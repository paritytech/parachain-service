//! Cross-parachain preimage registry (spec §3.1, §6.1).
//!
//! JAM allows only one `(hash, len)` solicitation per service, so the service
//! multiplexes: each entry records the set of `ParaId`s referencing the
//! preimage. JAM `solicit` fires on the empty→non-empty transition, JAM
//! `forget` on the reverse. The registry is keyed by `(hash, len)`: the same
//! hash at a different length is a distinct preimage.

use crate::state::{self, Tag};
use alloc::collections::BTreeSet;
use codec::{Decode, Encode};
use parachain_service_interface::types::{Hash, ParaId};

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
	pub fn get(hash: &Hash, len: u32) -> Option<PreimageEntry> {
		state::read(Tag::PreimageRegistry, &(*hash, len))
	}

	pub fn set(hash: &Hash, len: u32, entry: &PreimageEntry) {
		state::write(Tag::PreimageRegistry, &(*hash, len), entry)
	}

	pub fn remove(hash: &Hash, len: u32) {
		state::clear(Tag::PreimageRegistry, &(*hash, len))
	}

	/// Is `para_id` currently referencing `(hash, len)`?
	pub fn has_referencer(hash: &Hash, len: u32, para_id: ParaId) -> bool {
		Self::get(hash, len).is_some_and(|e| e.referencers.contains(&para_id))
	}
}
