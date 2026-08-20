//! PVF code-upgrade lifecycle (spec §5.2).
//!
//! Phase 1 Request: the PVF emits `RequestCodeUpgrade(hash, len)`.
//! Phase 2 Solicit + `pending_upgrade` arming (this module).
//! Phase 3 Preimage submitted out-of-band by anyone.
//! Phase 4 Dual-code window — Refine accepts either active or pending hash.
//! Phase 5 Activation on the first candidate validated with the new hash, OR
//!         lazy reap once the deadline passes (§5.1 step 4).

use crate::{
	constants::UPGRADE_TIMEOUT_TIMESLOTS,
	state::{
		log::{AccumulateLog, InsufficientBalanceReason},
		para_info::{ParaInfo, Parachains, ValidationCode},
	},
	state_balance,
};
use alloc::vec::Vec;
use jam_types::Slot;
use parachain_service_interface::types::{ParaId, ValidationCodeHash, ValidationCodeRef};

/// Release a code's referencer slot for `para_id`, UNLESS the parachain is
/// independently soliciting it (`pinned`). See §5.2.
fn release_code_if_not_pinned(
	para_id: ParaId,
	code: &ValidationCode,
	now: Slot,
	logs: &mut Vec<AccumulateLog>,
) {
	if code.pinned {
		return;
	}
	let out =
		state_balance::remove_referencer(para_id, &code.code_ref.hash.0, code.code_ref.len, now);
	logs.extend(out.log);
}

/// Phase 2: solicit the new code (if not already solicited) and arm
/// `pending_upgrade` with a deadline. Runs on-chain while replaying the
/// `RequestCodeUpgrade` upward message.
pub fn request_code_upgrade(
	para_id: ParaId,
	now: Slot,
	new_hash: ValidationCodeHash,
	code_len: u32,
	logs: &mut Vec<AccumulateLog>,
) {
	let pi = Parachains::get(para_id).expect("origin is live per step 1; qed");
	let deadline = now + UPGRADE_TIMEOUT_TIMESLOTS;

	// Requesting the already-active code is a no-op.
	// TODO: hash-only comparison per the Quint model, although the registry is
	// keyed by (hash, len) — same hash at a different len slips through. Needs
	// upstreaming.
	if pi.validation_code.as_ref().is_some_and(|vc| vc.code_ref.hash == new_hash) {
		return;
	}

	// Re-request of the same pending code: refresh the deadline only,
	// preserving the pending code's pinned bit.
	if let Some((pending, _)) = &pi.pending_upgrade {
		if pending.code_ref.hash == new_hash {
			let pending = pending.clone();
			let mut pi = pi;
			pi.pending_upgrade = Some((pending, deadline));
			// A backstop write failure (§6.1 invariant, SPEC_GAPS #4) logs the
			// rejection; the request is dropped for this block.
			if Parachains::set(para_id, &pi).is_err() {
				logs.push(AccumulateLog::InsufficientStateBalance {
					reason: InsufficientBalanceReason::ParaInfo,
				});
			}
			return;
		}
	}

	// The parachain independently solicited this code already iff it is a
	// referencer (it is neither the active nor the current pending code here).
	let parachain_had_it = crate::state::preimage_registry::PreimageRegistry::has_referencer(
		&new_hash.0,
		code_len,
		para_id,
	);

	if let Err(log) = state_balance::add_referencer(para_id, &new_hash.0, code_len) {
		// Insufficient balance; pending untouched.
		logs.push(log);
		return;
	}

	// Supersede a different in-flight upgrade: release it unless pinned.
	if let Some((old_pending, _)) = &pi.pending_upgrade {
		release_code_if_not_pinned(para_id, old_pending, now, logs);
	}

	let mut pi = Parachains::get(para_id).expect("still live; qed");
	pi.pending_upgrade = Some((
		ValidationCode {
			code_ref: ValidationCodeRef { hash: new_hash, len: code_len },
			pinned: parachain_had_it,
		},
		deadline,
	));
	// Arming `pending_upgrade` grows the record; a backstop write failure (§6.1
	// invariant, SPEC_GAPS #4) logs the rejection and the upgrade is not armed.
	if Parachains::set(para_id, &pi).is_err() {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::ParaInfo,
		});
	}
}

/// Whether `pi`'s pending upgrade has passed its deadline and so must be treated
/// as already gone when this candidate's validation code is checked (§5.1 step 4).
///
/// Kept separate from [`reap_timed_out_upgrade`] so step 5 can consult the
/// post-reap view without writing anything: a candidate rejected at step 5 must
/// leave `pending_upgrade` untouched.
pub fn pending_upgrade_expired(pi: &ParaInfo, now: Slot) -> bool {
	pi.pending_upgrade.as_ref().is_some_and(|(_, deadline)| *deadline <= now)
}

/// Phase 5(b): lazy reap on the next per-work-package accumulate (§5.1 step 4).
/// If the deadline has passed, release the pending code (unless pinned) and
/// clear `pending_upgrade`.
pub fn reap_timed_out_upgrade(para_id: ParaId, now: Slot, logs: &mut Vec<AccumulateLog>) {
	let pi = Parachains::get(para_id).expect("checked live; qed");
	if !pending_upgrade_expired(&pi, now) {
		return;
	}
	let Some((pending, _)) = &pi.pending_upgrade else { return };
	let pending = pending.clone();
	release_code_if_not_pinned(para_id, &pending, now, logs);
	let mut pi = Parachains::get(para_id).expect("still live; qed");
	pi.pending_upgrade = None;
	// Clearing `pending_upgrade` shrinks the record; JAM never rejects it.
	Parachains::set(para_id, &pi).expect("clearing pending_upgrade shrinks the record; qed");
}

/// Phase 5(a): activate the new code when this candidate was validated with the
/// pending hash (§5.1 step 6). The old active code's referencer is released
/// unless pinned; the pending code's `pinned` bit carries over.
pub fn activate_upgrade_if_match(
	para_id: ParaId,
	candidate_code_hash: ValidationCodeHash,
	now: Slot,
	logs: &mut Vec<AccumulateLog>,
) {
	let pi = Parachains::get(para_id).expect("checked live; qed");
	let Some((pending, _)) = &pi.pending_upgrade else { return };
	if candidate_code_hash != pending.code_ref.hash {
		// Validated with the old code; pending stays armed.
		return;
	}
	let pending = pending.clone();
	if let Some(active) = &pi.validation_code {
		release_code_if_not_pinned(para_id, &active.clone(), now, logs);
	}
	let mut pi: ParaInfo = Parachains::get(para_id).expect("still live; qed");
	pi.validation_code = Some(pending);
	pi.pending_upgrade = None;
	// Activation can grow the record; a backstop write failure (§6.1 invariant,
	// SPEC_GAPS #4) logs the rejection and the activation is deferred.
	if Parachains::set(para_id, &pi).is_err() {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::ParaInfo,
		});
	}
}
