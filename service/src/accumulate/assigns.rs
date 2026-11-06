//! Scheduled JAM `assign`s: caching, inline application, and the due-assign
//! flushes (spec §5.1, §7.1).

use crate::{
	constants::{AUTHORIZER_QUEUE_LEN, DUE_ASSIGN_FLUSH_BASE_GAS, DUE_ASSIGN_GAS},
	state::{
		assigns::{DirtyCores, PendingAssign, PendingAssigns},
		log::AccumulateLog,
	},
};
use alloc::vec::Vec;
use jam_pvm_common::{accumulate::assign, ApiError};
use jam_types::{
	auth_queue_len, core_count, AuthQueue, AuthorizerHash as JamAuthorizerHash, UnsignedGas,
};
use parachain_service_core::types::{AuthorizerHash, CoreIndex, ServiceId, Timeslot};

/// Replay an `AssignCore` message (Coretime only, §4.3). Refine rejects a
/// malformed queue or an unknown core (§3.3), so either is a defensive no-op
/// here. An already-due `jam_slot` applies inline; otherwise the entry is cached
/// until its slot, replacing any entry cached for `core`.
pub fn schedule(
	now: Timeslot,
	service_id: ServiceId,
	core: CoreIndex,
	queue: Vec<AuthorizerHash>,
	new_assigner: Option<ServiceId>,
	jam_slot: Timeslot,
	logs: &mut Vec<AccumulateLog>,
) {
	if !well_formed(&queue, new_assigner) || core >= core_count() {
		return;
	}
	if jam_slot <= now {
		match jam_assign(service_id, core, &queue, new_assigner, jam_slot) {
			Ok(()) => settle_after_assign(core, queue, new_assigner, jam_slot),
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
	cache(core, queue, new_assigner, jam_slot);
}

/// Keep the payload and due time together when either storage write is rejected.
fn cache(
	core: CoreIndex,
	queue: Vec<AuthorizerHash>,
	new_assigner: Option<ServiceId>,
	jam_slot: Timeslot,
) {
	let previous = PendingAssigns::get(core);
	if PendingAssigns::set(core, &PendingAssign { queue, assigner: new_assigner }).is_err() {
		// Keep the prior payload and due time together. Arming the index after
		// a rejected new payload could make the due flush read a missing entry.
		jam_pvm_common::error!("assign for core {core} not cached: storage full");
		return;
	}
	if DirtyCores::upsert(core, jam_slot).is_err() {
		// The index kept its old contents. Restore the matching payload; the
		// rejected index write spent nothing, so its original footprint fits.
		match previous {
			Some(entry) => PendingAssigns::set(core, &entry)
				.expect("restoring the original payload fits its original deposit; qed"),
			None => PendingAssigns::remove(core),
		}
		jam_pvm_common::error!("dirty-core index not updated for core {core}: storage full");
	}
}

/// How many due assigns `gas` pays for in one flush (§5.1).
pub fn budget(gas: UnsignedGas) -> usize {
	(gas.saturating_sub(DUE_ASSIGN_FLUSH_BASE_GAS) / DUE_ASSIGN_GAS) as usize
}

/// Flush up to `budget` due pending assigns (§5.1): earliest due timeslot first,
/// then lowest core index, so ones carried over from an earlier block go before
/// newer ones. Due cores beyond the budget stay cached for a later flush. Gating
/// reads only the dirty-core index; the payload is read just for fired cores.
///
/// Returns the rejections to record in the Coretime chain's log.
pub fn apply_due_assigns(
	now: Timeslot,
	service_id: ServiceId,
	budget: usize,
) -> Vec<AccumulateLog> {
	let mut logs = Vec::new();
	let cores = DirtyCores::get();
	let mut due: Vec<_> = cores.iter().copied().filter(|(_, jam_slot)| *jam_slot <= now).collect();
	if due.is_empty() || budget == 0 {
		return logs;
	}
	due.sort_unstable_by_key(|&(core, jam_slot)| (jam_slot, core));
	due.truncate(budget);
	let mut next = cores;
	next.retain(|entry| !due.contains(entry));
	for (core, jam_slot) in due {
		let entry = PendingAssigns::get(core).expect("dirty index names cached entries; qed");
		match jam_assign(service_id, core, &entry.queue, entry.assigner, jam_slot) {
			Ok(()) if fills_directly(entry.queue.len()) => PendingAssigns::remove(core),
			// The set stays cached as sent and is rewritten 80 slots after this one was
			// due, even when it is applied late (§7.1).
			Ok(()) => next
				.try_push((core, jam_slot + auth_queue_len() as Timeslot))
				.expect("re-arming cannot exceed the original number of dirty cores; qed"),
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
	// Flushing due cores never grows the index; JAM never rejects it.
	DirtyCores::set(&next).expect("flushing due cores never grows the index; qed");
	logs
}

/// §3.3: a queue holds 1 to `AUTHORIZER_QUEUE_LEN` hashes, and one handing the
/// core to another service holds exactly `AUTHORIZER_QUEUE_LEN`.
fn well_formed(queue: &[AuthorizerHash], new_assigner: Option<ServiceId>) -> bool {
	match new_assigner {
		None => (1..=AUTHORIZER_QUEUE_LEN).contains(&queue.len()),
		Some(_) => queue.len() == AUTHORIZER_QUEUE_LEN,
	}
}

/// Drop a self-sufficient queue after it fires, or keep a non-tiling set as sent
/// so it is rewritten 80 slots after `due` (§7.1).
fn settle_after_assign(
	core: CoreIndex,
	queue: Vec<AuthorizerHash>,
	assigner: Option<ServiceId>,
	due: Timeslot,
) {
	if fills_directly(queue.len()) {
		PendingAssigns::remove(core);
		DirtyCores::remove(core);
	} else {
		cache(core, queue, assigner, due + auth_queue_len() as Timeslot);
	}
}

fn fills_directly(queue_len: usize) -> bool {
	auth_queue_len() % queue_len == 0
}

/// Call JAM `assign(core, queue, assigner)` for an assign due at `due`. JAM adds
/// entry `t % 80` to the pool at slot `t`, and slot `t` gets hash `t % len` of
/// the set, so the slots `due` to `due + 79` are written into entries `due % 80`
/// onwards, wrapping to entry 0 (§7.1, DECISIONS.md D-7). A set that tiles the
/// 80 entries comes out as `queue` repeated from entry 0. A cached `assigner` of
/// `None` resolves to this service — JAM always writes one. JAM answers
/// `ActionInvalid` once this service is no longer the core's assigner.
fn jam_assign(
	service_id: ServiceId,
	core: CoreIndex,
	queue: &[AuthorizerHash],
	assigner: Option<ServiceId>,
	due: Timeslot,
) -> Result<(), ApiError> {
	let target_len = auth_queue_len() as u64;
	let len = queue.len() as u64;
	// Entry `due % 80` holds slot `due`; the entries before it hold the slots
	// after `due + 80 - due % 80`, so entry 0 holds that slot.
	let first_entry = due as u64 % target_len;
	let mut hash = ((due as u64 + (target_len - first_entry) % target_len) % len) as usize;
	let expanded: Vec<JamAuthorizerHash> = (0..target_len)
		.map(|entry| {
			if entry == first_entry {
				hash = (due as u64 % len) as usize;
			}
			let authorizer = JamAuthorizerHash(queue[hash]);
			hash = if hash + 1 == queue.len() { 0 } else { hash + 1 };
			authorizer
		})
		.collect();
	let auth_queue = AuthQueue::try_from(expanded).expect("expanded to the exact length; qed");
	assign(core, &auth_queue, assigner.unwrap_or(service_id))
}
