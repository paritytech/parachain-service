//! Coretime-chain-only host calls for the parachain lifecycle (spec §6).
//!
//! All four are idempotent; the service performs no rights-checking of its own
//! beyond the Refine-side origin restriction (§4.3, D-2) — the Coretime chain is
//! the sole authority on which ParaIds are live and who owns them.
//! ParaInfo is baseline-covered: host write failures leave the old entry intact
//! without an insufficient-allowance log (§6.1).

use crate::{
	head_commitment::HeadTracker,
	state::{
		log::{AccumulateLog, ParachainLogs, StateBalanceRejection},
		para_info::{ParaInfo, Parachains},
		validator_keys::StagedValidatorKeys,
	},
	state_balance::{add_referencer, baseline_for, clean_up_allowed_balance, remove_referencer},
};
use alloc::vec::Vec;
use jam_types::Slot;
use parachain_service_core::types::{
	validation_code_hash_bytes, Balance, HeadData, ParaId, ValidationCodeRef, ASSET_HUB_PARA_ID,
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
					para_id,
					attempted: new_total.into(),
					reason: StateBalanceRejection::BelowUsed {
						current_total: 0u64.into(),
						current_used: baseline.into(),
					},
				});
				return;
			}
			// Registration gives the para its first head, which §5.5 counts as a
			// change; the existing-para arm below touches no head.
			heads.touch(para_id);
			let _ = Parachains::set(
				para_id,
				&ParaInfo {
					head_data: HeadData::default(),
					validation_code: None,
					announced_upgrade: None,
					total_state_balance: new_total,
					used_state_balance: baseline,
					is_deregistering: false,
				},
			);
		},
		Some(mut pi) => {
			if pi.is_deregistering {
				logs.push(AccumulateLog::StateBalanceUpdateRejected {
					para_id,
					attempted: new_total.into(),
					reason: StateBalanceRejection::ParachainIsDeregistering,
				});
				return;
			}
			if new_total < pi.used_state_balance {
				// The Coretime chain cannot strand currently-paid-for state.
				logs.push(AccumulateLog::StateBalanceUpdateRejected {
					para_id,
					attempted: new_total.into(),
					reason: StateBalanceRejection::BelowUsed {
						current_total: pi.total_state_balance.into(),
						current_used: pi.used_state_balance.into(),
					},
				});
				return;
			}
			pi.total_state_balance = new_total;
			let _ = Parachains::set(para_id, &pi);
		},
	}
}

/// §6.2/§6.3 — upsert head data. No-op on an unregistered ParaId (Coretime must
/// call `parachain_set_state_balance` first).
pub fn set_head(para_id: ParaId, new_head: HeadData, heads: &mut HeadTracker) {
	let Some(mut pi) = Parachains::get(para_id) else { return };
	if pi.is_deregistering {
		return;
	}
	heads.touch(para_id);
	pi.head_data = new_head;
	let _ = Parachains::set(para_id, &pi);
}

/// §6.2/§6.3 — upsert validation code, bypassing the normal upgrade lifecycle
/// (forced replacement). Solicits the new code and clears the announcement. The
/// displaced codes are left untouched, exactly as an `Apply` leaves them: they
/// stay ordinary solicited preimages until the para (or Coretime, via a delegated
/// `Forget`) releases them.
pub fn set_validation_code(
	para_id: ParaId,
	new_code: ValidationCodeRef,
	logs: &mut Vec<AccumulateLog>,
) {
	let Some(pi) = Parachains::get(para_id) else { return };
	if pi.is_deregistering {
		return;
	}

	// Acquire the new referencer (no charge if already solicited); reject the
	// whole call if there is no headroom.
	if let Err(log) =
		add_referencer(para_id, &validation_code_hash_bytes(&new_code.hash), new_code.len)
	{
		logs.push(log);
		return;
	}

	let mut updated = Parachains::get(para_id).expect("still live; qed");
	updated.validation_code = Some(new_code);
	updated.announced_upgrade = None;
	let _ = Parachains::set(para_id, &updated);
}

/// §6.4 — deregister a parachain. Rejects with `TooMuchStateHeld` unless the
/// para holds only its baseline plus validation code(s). Forgets the codes via
/// the two-step forget; if any cannot be expunged yet, sets `is_deregistering`
/// and stops (Coretime retries once strictly past the logged `due`). Once every
/// code is expunged, drops all per-para state.
pub fn clean_up(
	para_id: ParaId,
	now: Slot,
	logs: &mut Vec<AccumulateLog>,
	heads: &mut HeadTracker,
) {
	let Some(pi) = Parachains::get(para_id) else { return };

	if pi.used_state_balance > clean_up_allowed_balance(&pi, para_id) {
		// Still holds solicited preimages or KV entries beyond the baseline —
		// they must be released first (by the para or by Coretime via the
		// para_id-taking `forget`/`kv_remove`, §6.4).
		logs.push(AccumulateLog::TooMuchStateHeld);
		return;
	}

	let mut retained = false;
	for code_ref in [pi.validation_code, pi.announced_upgrade].into_iter().flatten() {
		let out = remove_referencer(
			para_id,
			&validation_code_hash_bytes(&code_ref.hash),
			code_ref.len,
			now,
		);
		retained |= out.retained;
		logs.extend(out.log);
	}

	if retained {
		// Some code awaits its second, expunging forget: keep the entry and
		// reject all further work packages for this para (§5.1 step 1).
		let mut pi = Parachains::get(para_id).expect("still live; qed");
		pi.is_deregistering = true;
		let _ = Parachains::set(para_id, &pi);
		return;
	}

	// Fully expunged — drop all per-para state. `key_value_storage` is
	// necessarily empty here: any entry would raise `used_state_balance` above
	// the allowed clean-up balance checked above (JAM storage has no prefix
	// iteration, so a sweep would be impossible anyway).
	// Preserve the pre-block head if Coretime re-registers this para later in
	// the same block. Registration must not mistake its prior head for absent.
	heads.touch(para_id);
	Parachains::remove(para_id);
	ParachainLogs::remove(para_id);
	if para_id == ASSET_HUB_PARA_ID {
		StagedValidatorKeys::clear();
	}
}
