//! Scheduled-but-unapplied JAM `assign` payloads (spec §3.1, §5.1, §7.1).

use crate::{constants::CORE_COUNT, state, state::Tag};
use alloc::vec::Vec;
use bounded_collections::{BoundedVec, ConstU32};
use codec::{Decode, Encode};
use parachain_service_interface::types::{AuthorizerHash, CoreIndex, ServiceId, Timeslot};

/// A scheduled JAM `assign` for one core (spec §3.1, §7.1). The queue is
/// stored rotated to the beginning of its next 80-slot cycle, so no separate
/// cursor is needed. JAM writes the queue and assigner atomically.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct PendingAssign {
	pub queue: Vec<AuthorizerHash>,
	/// `None` keeps this service as the core's assigner; `Some(s)` hands the
	/// core to `s` (one-way).
	pub assigner: Option<ServiceId>,
}

/// Dirty-core index: each core with a pending assign, paired with its next due
/// timeslot. A non-tiling short queue is re-armed every 80 slots.
pub type PendingAssignCores = BoundedVec<(CoreIndex, Timeslot), ConstU32<{ CORE_COUNT as u32 }>>;

/// Storage accessors for the `pending_assigns` map (tag `0x02`).
pub struct PendingAssigns;

impl PendingAssigns {
	pub fn get(core: CoreIndex) -> Option<PendingAssign> {
		state::read(Tag::PendingAssigns, &core)
	}

	pub fn set(core: CoreIndex, assign: &PendingAssign) {
		state::write(Tag::PendingAssigns, &core, assign)
	}

	pub fn remove(core: CoreIndex) {
		state::clear(Tag::PendingAssigns, &core)
	}
}

/// Storage accessors for the `pending_assign_cores` singleton (tag `0x03`).
pub struct DirtyCores;

impl DirtyCores {
	pub fn get() -> PendingAssignCores {
		state::read_singleton(Tag::PendingAssignCores).unwrap_or_default()
	}

	pub fn set(cores: &PendingAssignCores) {
		state::write_singleton(Tag::PendingAssignCores, cores)
	}

	/// Upsert the `(core, jam_slot)` pair. Panics if more than [`CORE_COUNT`]
	/// cores are dirty, which is impossible: the index is keyed by core.
	pub fn upsert(core: CoreIndex, jam_slot: Timeslot) {
		let mut cores = Self::get();
		if let Some(entry) = cores.iter_mut().find(|(c, _)| *c == core) {
			entry.1 = jam_slot;
		} else {
			cores
				.try_push((core, jam_slot))
				.expect("at most CORE_COUNT distinct cores can be dirty; qed");
		}
		Self::set(&cores);
	}

	pub fn remove(core: CoreIndex) {
		let mut cores = Self::get();
		cores.retain(|(c, _)| *c != core);
		Self::set(&cores);
	}
}
