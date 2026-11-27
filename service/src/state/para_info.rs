//! Per-parachain record (spec §3.1) and its storage accessors.

use crate::state::{self, StorageFull, Tag};
use parachain_service_core::types::ParaId;
pub use parachain_service_core::para_info::ParaInfo;

/// Storage accessors for the `parachains` map (tag `0x00`).
pub struct Parachains;

impl Parachains {
	pub fn get(para_id: ParaId) -> Option<ParaInfo> {
		state::read(&Tag::Parachains.para_key(para_id))
	}

	/// Upsert a para record. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(para_id: ParaId, info: &ParaInfo) -> Result<(), StorageFull> {
		state::write(&Tag::Parachains.para_key(para_id), info)
	}

	pub fn remove(para_id: ParaId) {
		state::clear(&Tag::Parachains.para_key(para_id))
	}

	/// §5.1 step 1: a para is live iff registered.
	pub fn is_live(para_id: ParaId) -> bool {
		Self::get(para_id).is_some()
	}
}
