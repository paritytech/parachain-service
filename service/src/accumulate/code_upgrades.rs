//! PVF code-upgrade lifecycle (spec §5.2).
//!
//! The parachain drives the switch itself with two upward messages: an
//! `Announcement` declares the code a later `Apply` may activate. Neither
//! carries a deadline — an announcement stays standing until applied or
//! superseded (§5.2).

use crate::state::{
	log::{AccumulateLog, InsufficientBalanceReason},
	para_info::Parachains,
	preimage_registry::PreimageRegistry,
};
use alloc::vec::Vec;
use parachain_service_core::types::{ParaId, Timeslot, ValidationCodeHash, ValidationCodeRef};

/// §5.2 `announceCodeUpgrade`: declare the code a later `Apply` may switch to.
/// Replayed from a `RequestCodeUpgrade` upward message during Accumulate.
/// Availability is judged as of `lookup_anchor`, the anchor of the work report
/// carrying the message.
pub fn announce_code_upgrade(
	para_id: ParaId,
	new_hash: ValidationCodeHash,
	code_len: u32,
	lookup_anchor: Timeslot,
	logs: &mut Vec<AccumulateLog>,
) {
	let pi = Parachains::get(para_id).expect("origin is live per step 1; qed");
	let new_ref = ValidationCodeRef { hash: new_hash, len: code_len };

	// Announcing the running code is a no-op, so an announcement never shadows
	// the active code.
	if pi.validation_code == Some(new_ref) {
		return;
	}

	// The code must be referenced by this para AND available for lookup at the
	// work report's lookup anchor.
	let referenced = PreimageRegistry::has_referencer(&new_hash.0, code_len, para_id);
	if !referenced || !PreimageRegistry::is_available_at(&new_hash.0, code_len, lookup_anchor) {
		logs.push(AccumulateLog::CodeUpgradeNotAvailable {
			hash: new_hash.0,
			len: code_len.into(),
		});
		return;
	}

	// A superseded announcement stops being validation code, so it simply
	// unpins: its referencer is the parachain's own solicit, which stands until
	// the parachain forgets it. No jamForget is issued.
	let mut pi = pi;
	pi.announced_upgrade = Some(new_ref);
	if Parachains::set(para_id, &pi).is_err() {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::ParaInfo,
		});
	}
}

/// §5.2 `applyCodeUpgrade`: switch `validation_code` to the announced code.
pub fn apply_code_upgrade(
	para_id: ParaId,
	new_hash: ValidationCodeHash,
	code_len: u32,
	logs: &mut Vec<AccumulateLog>,
) {
	let pi = Parachains::get(para_id).expect("origin is live per step 1; qed");
	let new_ref = ValidationCodeRef { hash: new_hash, len: code_len };

	if pi.announced_upgrade != Some(new_ref) {
		logs.push(AccumulateLog::CodeUpgradeNotAnnounced {
			hash: new_hash.0,
			len: code_len.into(),
		});
		return;
	}

	// The displaced active code stops being validation code, so it simply
	// unpins: its referencer stands until the parachain forgets it. No
	// jamForget is issued.
	let mut pi = pi;
	pi.validation_code = Some(new_ref);
	pi.announced_upgrade = None;
	if Parachains::set(para_id, &pi).is_err() {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::ParaInfo,
		});
	}
}
