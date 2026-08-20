//! Scheduled JAM `assign`s: caching, inline application, and the
//! always-accumulate flush (spec §5.1, §7.1).

use crate::state::assigns::{DirtyCores, PendingAssign, PendingAssigns};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::assign;
use jam_types::{auth_queue_len, AuthQueue, AuthorizerHash as JamAuthorizerHash};
use parachain_service_interface::types::{AuthorizerHash, CoreIndex, ServiceId, Timeslot};

/// Replay an `AssignCore` message (Coretime only, §4.3). An empty queue cancels
/// any cached entry (no JAM call); an already-due `jam_slot` applies inline
/// (always-accumulate has already run this block); otherwise the entry is
/// cached until its slot.
pub fn schedule(
	now: Timeslot,
	service_id: ServiceId,
	core: CoreIndex,
	queue: Vec<AuthorizerHash>,
	new_assigner: Option<ServiceId>,
	jam_slot: Timeslot,
) {
	if queue.is_empty() {
		PendingAssigns::remove(core);
		DirtyCores::remove(core);
		return;
	}
	if jam_slot <= now {
		jam_assign(service_id, core, &queue, new_assigner);
		PendingAssigns::remove(core);
		DirtyCores::remove(core);
		return;
	}
	PendingAssigns::set(core, &PendingAssign { queue, assigner: new_assigner }).unwrap_or_else(
		|_| {
			// A failed cache write (baseline-covered, §6.1 backstop, SPEC_GAPS #4)
			// drops the assign. There is no per-para log channel for the
			// service-global assign cache (F-15, SPEC_GAPS #9/#10), so only the
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
pub fn apply_due_assigns(now: Timeslot, service_id: ServiceId) {
	let cores = DirtyCores::get();
	if cores.is_empty() {
		return;
	}
	let mut survivors = cores.clone();
	survivors.retain(|(_, jam_slot)| now < *jam_slot);
	if survivors.len() == cores.len() {
		return;
	}
	for (core, jam_slot) in cores.iter() {
		if now < *jam_slot {
			continue;
		}
		let entry = PendingAssigns::get(*core).expect("dirty index names cached entries; qed");
		jam_assign(service_id, *core, &entry.queue, entry.assigner);
		PendingAssigns::remove(*core);
	}
	// Flushing the due cores shrinks the index; JAM never rejects it.
	DirtyCores::set(&survivors).expect("flushing due cores shrinks the index; qed");
}

/// Call JAM `assign(core, queue, assigner)`. A queue shorter than the protocol's
/// exact length is cycle-repeated (`queue[i mod len]`, DECISIONS.md D-7); a
/// cached `assigner` of `None` resolves to this service — JAM always writes one.
fn jam_assign(
	service_id: ServiceId,
	core: CoreIndex,
	queue: &[AuthorizerHash],
	assigner: Option<ServiceId>,
) {
	let target_len = auth_queue_len();
	let expanded: Vec<JamAuthorizerHash> =
		(0..target_len).map(|i| JamAuthorizerHash(queue[i % queue.len()])).collect();
	let auth_queue = AuthQueue::try_from(expanded).expect("expanded to the exact length; qed");
	if let Err(e) = assign(core, &auth_queue, assigner.unwrap_or(service_id)) {
		// TODO: no AccumulateLog is specified for a failed assign (bad core, or
		// the service is no longer the core's assigner after a hand-off);
		// needs upstreaming (SPEC_GAPS #9/#10).
		jam_pvm_common::error!("assign for core {core} failed: {e:?}");
	}
}
