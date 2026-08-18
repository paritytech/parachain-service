//! Coretime-chain-only host calls for the parachain lifecycle (spec §6).
//!
//! All four are idempotent; the service performs no rights-checking of its own
//! beyond the Refine-side origin restriction (§4.3, D-2) — the Coretime chain is
//! the sole authority on which ParaIds are live and who owns them.

use crate::{
	head_commitment::HeadTracker,
	state::{
		log::{AccumulateLog, ParachainLogs},
		para_info::{ParaInfo, Parachains, ValidationCode},
		preimage_registry::PreimageRegistry,
		validator_keys::StagedValidatorKeys,
	},
	state_balance::{add_referencer, baseline_for, clean_up_allowed_balance, remove_referencer},
};
use alloc::vec::Vec;
use jam_types::Slot;
use parachain_service_interface::types::{
	Balance, HeadData, ParaId, ValidationCodeHash, ValidationCodeRef, ASSET_HUB_PARA_ID,
};

/// §6.1 — the sole creator of `ParaInfo`. On an unused ParaId, creates the entry
/// with the baseline footprint pre-charged; on an existing one, overwrites
/// `total_state_balance` iff `new_total >= used_state_balance`.
pub fn set_state_balance(
	para_id: ParaId,
	new_total: Balance,
	logs: &mut Vec<AccumulateLog>,
	heads: &mut HeadTracker,
) {
	match Parachains::get(para_id) {
		None => {
			let baseline = baseline_for(para_id);
			if new_total < baseline {
				logs.push(AccumulateLog::StateBalanceUpdateRejected {
					attempted: new_total.into(),
					current_total: 0u64.into(),
					current_used: baseline.into(),
				});
				return;
			}
			// Registration gives the para its first head, which §5.5 counts as a
			// change; the existing-para arm below touches no head.
			heads.touch(para_id);
			Parachains::set(
				para_id,
				&ParaInfo {
					head_data: HeadData::default(),
					validation_code: None,
					pending_upgrade: None,
					total_state_balance: new_total,
					used_state_balance: baseline,
					is_deregistering: false,
				},
			);
		},
		Some(mut pi) => {
			if new_total < pi.used_state_balance {
				// The Coretime chain cannot strand currently-paid-for state.
				logs.push(AccumulateLog::StateBalanceUpdateRejected {
					attempted: new_total.into(),
					current_total: pi.total_state_balance.into(),
					current_used: pi.used_state_balance.into(),
				});
				return;
			}
			pi.total_state_balance = new_total;
			Parachains::set(para_id, &pi);
		},
	}
}

/// §6.2/§6.3 — upsert head data. No-op on an unregistered ParaId (Coretime must
/// call `parachain_set_state_balance` first).
pub fn set_head(para_id: ParaId, new_head: HeadData, heads: &mut HeadTracker) {
	let Some(mut pi) = Parachains::get(para_id) else { return };
	heads.touch(para_id);
	pi.head_data = new_head;
	Parachains::set(para_id, &pi);
}

/// §6.2/§6.3 — upsert validation code, bypassing the normal upgrade lifecycle
/// (forced replacement). Solicits the new code, releases the displaced active
/// and pending codes (each unless pinned or equal to the new code), and clears
/// the pending upgrade.
pub fn set_validation_code(
	para_id: ParaId,
	new_hash: ValidationCodeHash,
	code_len: u32,
	now: Slot,
	logs: &mut Vec<AccumulateLog>,
) {
	let Some(pi) = Parachains::get(para_id) else { return };

	// TODO: hash-only comparisons per the Quint model, although the registry is
	// keyed by (hash, len). Needs upstreaming.
	let active_equals_new =
		pi.validation_code.as_ref().is_some_and(|vc| vc.code_ref.hash == new_hash);
	let pending_equals_new =
		pi.pending_upgrade.as_ref().is_some_and(|(vc, _)| vc.code_ref.hash == new_hash);
	// The para independently solicited the new code iff it references it for a
	// reason other than being the current active or pending code.
	let parachain_had_it = PreimageRegistry::has_referencer(&new_hash.0, code_len, para_id) &&
		!active_equals_new &&
		!pending_equals_new;

	// Acquire the new referencer (no charge if already solicited); reject the
	// whole call if there is no headroom.
	if let Err(log) = add_referencer(para_id, &new_hash.0, code_len) {
		logs.push(log);
		return;
	}

	// Release the displaced active code, unless it equals the new one or the
	// para pinned it.
	if let Some(vc) = &pi.validation_code {
		if vc.code_ref.hash != new_hash && !vc.pinned {
			let out = remove_referencer(para_id, &vc.code_ref.hash.0, vc.code_ref.len, now);
			logs.extend(out.log);
		}
	}
	// Clear any pending upgrade, releasing its code under the same rule.
	if let Some((vc, _)) = &pi.pending_upgrade {
		if vc.code_ref.hash != new_hash && !vc.pinned {
			let out = remove_referencer(para_id, &vc.code_ref.hash.0, vc.code_ref.len, now);
			logs.extend(out.log);
		}
	}

	let mut updated = Parachains::get(para_id).expect("still live; qed");
	// If the new hash equals the existing active code, preserve its pinned bit;
	// otherwise the new code's bit records the para's own prior solicit.
	let pinned = match &pi.validation_code {
		Some(vc) if vc.code_ref.hash == new_hash => vc.pinned,
		_ => parachain_had_it,
	};
	updated.validation_code = Some(ValidationCode {
		code_ref: ValidationCodeRef { hash: new_hash, len: code_len },
		pinned,
	});
	updated.pending_upgrade = None;
	Parachains::set(para_id, &updated);
}

/// §6.4 — deregister a parachain. Rejects with `TooMuchStateHeld` unless the
/// para holds only its baseline plus validation code(s). Forgets the codes via
/// the two-step forget; if any cannot be expunged yet, sets `is_deregistering`
/// and stops (Coretime retries once strictly past the logged `due`). Once every
/// code is expunged, drops all per-para state.
pub fn clean_up(para_id: ParaId, now: Slot, logs: &mut Vec<AccumulateLog>) {
	let Some(pi) = Parachains::get(para_id) else { return };

	if pi.used_state_balance > clean_up_allowed_balance(&pi, para_id) {
		// Still holds solicited preimages or KV entries beyond the baseline —
		// they must be released first (by the para or by Coretime via the
		// para_id-taking `forget`/`kv_remove`, §6.4).
		logs.push(AccumulateLog::TooMuchStateHeld);
		return;
	}

	let mut retained = false;
	for code_ref in [
		pi.validation_code.as_ref().map(|vc| vc.code_ref),
		pi.pending_upgrade.as_ref().map(|(vc, _)| vc.code_ref),
	]
	.into_iter()
	.flatten()
	{
		let out = remove_referencer(para_id, &code_ref.hash.0, code_ref.len, now);
		retained |= out.retained;
		logs.extend(out.log);
	}

	if retained {
		// Some code awaits its second, expunging forget: keep the entry and
		// reject all further work packages for this para (§5.1 step 1).
		let mut pi = Parachains::get(para_id).expect("still live; qed");
		pi.is_deregistering = true;
		Parachains::set(para_id, &pi);
		return;
	}

	// Fully expunged — drop all per-para state. `key_value_storage` is
	// necessarily empty here: any entry would raise `used_state_balance` above
	// the allowed clean-up balance checked above (JAM storage has no prefix
	// iteration, so a sweep would be impossible anyway — SPEC_GAPS #13).
	Parachains::remove(para_id);
	ParachainLogs::remove(para_id);
	if para_id == ASSET_HUB_PARA_ID {
		StagedValidatorKeys::clear();
	}
}
