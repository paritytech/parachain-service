//! Scheduled JAM `assign`s: caching, inline application, and the
//! always-accumulate flush (spec §5.1, §7.1).

use crate::{
	constants::AUTHORIZER_QUEUE_LEN,
	state::{
		assigns::{DirtyCores, PendingAssign, PendingAssigns},
		log::AccumulateLog,
	},
};
use alloc::vec::Vec;
use jam_pvm_common::{accumulate::assign, ApiError};
use jam_types::{auth_queue_len, AuthQueue, AuthorizerHash as JamAuthorizerHash};
use parachain_service_core::types::{AuthorizerHash, CoreIndex, ServiceId, Timeslot};

/// Replay an `AssignCore` message (Coretime only, §4.3). Refine rejects a
/// malformed queue, so one is a defensive no-op here. An already-due `jam_slot`
/// applies inline (always-accumulate has already run this block); otherwise the
/// entry is cached until its slot.
pub fn schedule(
	now: Timeslot,
	service_id: ServiceId,
	core: CoreIndex,
	queue: Vec<AuthorizerHash>,
	new_assigner: Option<ServiceId>,
	jam_slot: Timeslot,
	logs: &mut Vec<AccumulateLog>,
) {
	if !well_formed(&queue, new_assigner) {
		return;
	}
	if jam_slot <= now {
		match jam_assign(service_id, core, &queue, new_assigner) {
			Ok(()) => settle_after_assign(core, queue, new_assigner, now),
			// The core was handed away: nothing is scheduled, and any entry still
			// cached for it is dropped (§7.1).
			Err(ApiError::ActionInvalid) => {
				PendingAssigns::remove(core);
				DirtyCores::remove(core);
				logs.push(AccumulateLog::CoreNotAssignable { core });
			},
			Err(e) => jam_pvm_common::error!("assign for core {core} failed: {e:?}"),
		}
		return;
	}
	PendingAssigns::set(core, &PendingAssign { queue, assigner: new_assigner }).unwrap_or_else(
		|_| {
			// A failed cache write (baseline-covered, §6.1 backstop)
			// drops the assign. There is no per-para log channel for the
			// service-global assign cache, so only the
			// error is surfaced. The dirty-core index must NOT be armed: the
			// flush would then expect a payload that was never cached.
			jam_pvm_common::error!("assign for core {core} not cached: storage full");
		},
	);
	if DirtyCores::upsert(core, jam_slot).is_err() {
		jam_pvm_common::error!("dirty-core index not updated for core {core}: storage full");
	}
}

/// The always-accumulate phase (§5.1): flush every due pending assign. Gating
/// reads only the dirty-core index; the payload is read just for due cores.
///
/// Returns the rejections to record in the Coretime chain's log.
pub fn apply_due_assigns(now: Timeslot, service_id: ServiceId) -> Vec<AccumulateLog> {
	let mut logs = Vec::new();
	let cores = DirtyCores::get();
	if cores.is_empty() {
		return logs;
	}
	let mut next = cores.clone();
	next.retain(|(_, jam_slot)| now < *jam_slot);
	if next.len() == cores.len() {
		return logs;
	}
	for (core, jam_slot) in cores {
		if now < jam_slot {
			continue;
		}
		let entry = PendingAssigns::get(core).expect("dirty index names cached entries; qed");
		match jam_assign(service_id, core, &entry.queue, entry.assigner) {
			Ok(()) if fills_directly(entry.queue.len()) => PendingAssigns::remove(core),
			Ok(()) => {
				let _ = PendingAssigns::set(
					core,
					&PendingAssign { queue: advance_queue(entry.queue), assigner: entry.assigner },
				);
				next.try_push((core, now + auth_queue_len() as Timeslot))
					.expect("re-arming cannot exceed the original number of dirty cores; qed");
			},
			// The core was handed away since the entry was cached: drop it (§5.1).
			Err(ApiError::ActionInvalid) => {
				PendingAssigns::remove(core);
				logs.push(AccumulateLog::CoreNotAssignable { core });
			},
			// TODO: the design does not say what happens to an entry JAM rejects
			// for any other reason; it is retried.
			Err(e) => {
				jam_pvm_common::error!("assign for core {core} failed: {e:?}");
				next.try_push((core, jam_slot))
					.expect("re-arming cannot exceed the original number of dirty cores; qed");
			},
		}
	}
	// Flushing the due cores shrinks the index; JAM never rejects it.
	DirtyCores::set(&next).expect("flushing due cores shrinks the index; qed");
	logs
}

/// §4.3: a queue holds 1 to `AUTHORIZER_QUEUE_LEN` hashes, and one handing the
/// core to another service holds exactly `AUTHORIZER_QUEUE_LEN`.
fn well_formed(queue: &[AuthorizerHash], new_assigner: Option<ServiceId>) -> bool {
	match new_assigner {
		None => (1..=AUTHORIZER_QUEUE_LEN).contains(&queue.len()),
		Some(_) => queue.len() == AUTHORIZER_QUEUE_LEN,
	}
}

/// Drop a self-sufficient queue after it fires, or retain and advance a short
/// non-tiling queue so its endless sequence resumes 80 slots later (§7.1).
fn settle_after_assign(
	core: CoreIndex,
	queue: Vec<AuthorizerHash>,
	assigner: Option<ServiceId>,
	now: Timeslot,
) {
	if fills_directly(queue.len()) {
		PendingAssigns::remove(core);
		DirtyCores::remove(core);
	} else {
		let _ = PendingAssigns::set(core, &PendingAssign { queue: advance_queue(queue), assigner });
		let _ = DirtyCores::upsert(core, now + auth_queue_len() as Timeslot);
	}
}

fn fills_directly(queue_len: usize) -> bool {
	auth_queue_len() % queue_len == 0
}

fn advance_queue(mut queue: Vec<AuthorizerHash>) -> Vec<AuthorizerHash> {
	let by = auth_queue_len() % queue.len();
	queue.rotate_left(by);
	queue
}

/// Call JAM `assign(core, queue, assigner)`. A queue shorter than the protocol's
/// exact length is cycle-repeated (`queue[i mod len]`, DECISIONS.md D-7); a
/// cached `assigner` of `None` resolves to this service — JAM always writes one.
/// JAM answers `ActionInvalid` once this service is no longer the core's
/// assigner.
fn jam_assign(
	service_id: ServiceId,
	core: CoreIndex,
	queue: &[AuthorizerHash],
	assigner: Option<ServiceId>,
) -> Result<(), ApiError> {
	let target_len = auth_queue_len();
	let expanded: Vec<JamAuthorizerHash> =
		(0..target_len).map(|i| JamAuthorizerHash(queue[i % queue.len()])).collect();
	let auth_queue = AuthQueue::try_from(expanded).expect("expanded to the exact length; qed");
	assign(core, &auth_queue, assigner.unwrap_or(service_id))
}
