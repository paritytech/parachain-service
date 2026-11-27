//! Cross-parachain preimage registry (spec §3.1, §6.1).
//!
//! JAM allows only one `(hash, len)` solicitation per service, so the service
//! multiplexes: each entry records the set of `ParaId`s referencing the
//! preimage. JAM `solicit` fires on the empty→non-empty transition, JAM
//! `forget` on the reverse. The registry is keyed by `(hash, len)`: the same
//! hash at a different length is a distinct preimage.

use crate::state::{self, StorageFull, Tag};
use alloc::collections::BTreeSet;
use codec::{Decode, Encode};
use jam_pvm_common::accumulate::{query, LookupRequestStatus};
use jam_types::Hash;
use parachain_service_core::types::{ParaId, Timeslot};

/// One registry entry: the parachains currently referencing this preimage.
// TODO: the spec bounds this by "the protocol-level maximum number of
// parachains", but no such constant is defined anywhere — unbounded for now.
// Needs upstreaming.
#[derive(Clone, Debug, Default, PartialEq, Eq, Encode, Decode)]
pub struct PreimageEntry {
	pub referencers: BTreeSet<ParaId>,
}

/// Storage accessors for the `preimage_registry` map (tag `0x04`).
///
/// Key encoding: `hash` (32 B) then `len` (fixed 4 B) — `|key| = 37` with the
/// tag, matching the §6.1 sizing.
pub struct PreimageRegistry;

impl PreimageRegistry {
	pub fn key(hash: &Hash, len: u32) -> [u8; 37] {
		let mut key = [0; 37];
		key[0] = Tag::PreimageRegistry as u8;
		key[1..33].copy_from_slice(hash);
		key[33..].copy_from_slice(&len.to_le_bytes());
		key
	}

	pub fn get(hash: &Hash, len: u32) -> Option<PreimageEntry> {
		state::read(&Self::key(hash, len))
	}

	/// Upsert an entry. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(hash: &Hash, len: u32, entry: &PreimageEntry) -> Result<(), StorageFull> {
		state::write(&Self::key(hash, len), entry)
	}

	pub fn remove(hash: &Hash, len: u32) {
		state::clear(&Self::key(hash, len))
	}

	/// Is `para_id` currently referencing `(hash, len)`?
	pub fn has_referencer(hash: &Hash, len: u32, para_id: ParaId) -> bool {
		Self::get(hash, len).is_some_and(|e| e.referencers.contains(&para_id))
	}

	/// §5.2 availability: whether JAM's `query(hash, len)` reports the preimage
	/// as provided or re-requested, and it became available at or before
	/// `anchor`. Mirrors the Quint model's `preimageAvailableAt`
	/// (code_upgrades.qnt).
	pub fn is_available_at(hash: &Hash, len: u32, anchor: Timeslot) -> bool {
		match query(hash, len as usize) {
			Some(LookupRequestStatus::Provided { since }) => since <= anchor,
			Some(LookupRequestStatus::Rerequested { rerequested_since, .. }) => {
				rerequested_since <= anchor
			},
			Some(LookupRequestStatus::Unrequested { .. }) |
			Some(LookupRequestStatus::Unprovided) |
			None => false,
		}
	}
}
