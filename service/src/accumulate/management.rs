//! Coretime-chain-only host calls for the parachain lifecycle (spec §6).
//!
//! All four are idempotent; the service performs no rights-checking of its own
//! beyond the Refine-side origin restriction (§4.3, D-2) — the Coretime chain is
//! the sole authority on which ParaIds are live and who owns them.

use crate::{
	head_commitment::HeadTracker,
	state::{
		log::{AccumulateLog, InsufficientBalanceReason, ParachainLogs, StateBalanceRejection},
		para_info::{ParaInfo, Parachains},
		validator_keys::StagedValidatorKeys,
	},
	state_balance::{add_referencer, baseline_for, clean_up_allowed_balance, remove_referencer},
};
use alloc::vec::Vec;
use jam_types::Slot;
use parachain_service_core::types::{
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
			// A failed registration write (backstop) logs the
			// rejection; the Coretime batch is not replayed.
			if Parachains::set(
				para_id,
				&ParaInfo {
					head_data: HeadData::default(),
					validation_code: None,
					announced_upgrade: None,
					total_state_balance: new_total,
					used_state_balance: baseline,
					is_deregistering: false,
				},
			)
			.is_err()
			{
				logs.push(AccumulateLog::InsufficientStateBalance {
					reason: InsufficientBalanceReason::ParaInfo,
				});
			}
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
			if Parachains::set(para_id, &pi).is_err() {
				logs.push(AccumulateLog::InsufficientStateBalance {
					reason: InsufficientBalanceReason::ParaInfo,
				});
			}
		},
	}
}

/// §6.2/§6.3 — upsert head data. No-op on an unregistered ParaId (Coretime must
/// call `parachain_set_state_balance` first).
pub fn set_head(
	para_id: ParaId,
	new_head: HeadData,
	heads: &mut HeadTracker,
	logs: &mut Vec<AccumulateLog>,
) {
	let Some(mut pi) = Parachains::get(para_id) else { return };
	if pi.is_deregistering {
		return;
	}
	heads.touch(para_id);
	pi.head_data = new_head;
	// A head overwrite can grow the `ParaInfo` entry; a backstop write failure
	// (§6.1 invariant) logs the rejection and accumulate continues.
	if Parachains::set(para_id, &pi).is_err() {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::ParaInfo,
		});
	}
}

/// §6.2/§6.3 — upsert validation code, bypassing the normal upgrade lifecycle
/// (forced replacement). Solicits the new code, releases the displaced active
/// and announced codes (each unless equal to the new code), and clears the
/// announcement.
pub fn set_validation_code(
	para_id: ParaId,
	new_hash: ValidationCodeHash,
	code_len: u32,
	now: Slot,
	logs: &mut Vec<AccumulateLog>,
) {
	let Some(pi) = Parachains::get(para_id) else { return };
	if pi.is_deregistering {
		return;
	}

	// Acquire the new referencer (no charge if already solicited); reject the
	// whole call if there is no headroom.
	if let Err(log) = add_referencer(para_id, &new_hash.0, code_len) {
		logs.push(log);
		return;
	}

	// TODO: hash-only comparisons per the Quint model, although the registry is
	// keyed by (hash, len). Needs upstreaming.
	// §6.3: release the displaced active code, unless it IS the new code.
	if let Some(vc) = &pi.validation_code {
		if vc.hash != new_hash {
			let out = remove_referencer(para_id, &vc.hash.0, vc.len, now);
			logs.extend(out.log);
		}
	}
	// Release the displaced announced code under the same rule.
	if let Some(vc) = &pi.announced_upgrade {
		if vc.hash != new_hash {
			let out = remove_referencer(para_id, &vc.hash.0, vc.len, now);
			logs.extend(out.log);
		}
	}

	let mut updated = Parachains::get(para_id).expect("still live; qed");
	updated.validation_code = Some(ValidationCodeRef { hash: new_hash, len: code_len });
	updated.announced_upgrade = None;
	// A forced-code write can grow the record; a backstop write failure (§6.1
	// invariant) logs the rejection.
	if Parachains::set(para_id, &updated).is_err() {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::ParaInfo,
		});
	}
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
		let out = remove_referencer(para_id, &code_ref.hash.0, code_ref.len, now);
		retained |= out.retained;
		logs.extend(out.log);
	}

	if retained {
		// Some code awaits its second, expunging forget: keep the entry and
		// reject all further work packages for this para (§5.1 step 1).
		let mut pi = Parachains::get(para_id).expect("still live; qed");
		pi.is_deregistering = true;
		// A backstop write failure (§6.1 invariant) leaves the para
		// live for one more block; logged and accumulate continues.
		if Parachains::set(para_id, &pi).is_err() {
			logs.push(AccumulateLog::InsufficientStateBalance {
				reason: InsufficientBalanceReason::ParaInfo,
			});
		}
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
