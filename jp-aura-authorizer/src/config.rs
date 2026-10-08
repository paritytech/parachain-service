use alloc::vec::Vec;
use codec::{Decode, Encode};
use jam_types::{ServiceId, Slot};
use primitive_types::H256;

use crate::ParaId;

#[derive(Debug, Encode, Decode)]
pub struct AuthConfig {
	/// Authoritative `ParaId` for each work item, in item order (§3.2). Must
	/// stay the config's first field: the Parachain Service's Refine decodes
	/// exactly this prefix.
	pub para_ids: Vec<ParaId>,
	/// The JAM service every work item must target. Prevents para-specific
	/// coretime being spent on other JAM work.
	/// TODO: not yet in the design's §7.1 config; needs upstreaming.
	pub parachain_service: ServiceId,
	/// Root of a binary Merkle tree over the collator public keys.
	/// Leaf index == collator index in the set.
	pub collator_set_root: H256,
	/// Number of collators in the set. Zero is rejected.
	pub collator_set_size: u32,
	/// Slot duration as a multiple of the JAM timeslot (6 s). Zero is rejected.
	pub slot_duration: u32,
}

/// §7.1 step 4 — the round-robin collator index expected for `slot`:
/// `(slot / slot_duration) mod collator_set_size`.
///
/// # Panics
///
/// Panics if `slot_duration` or `collator_set_size` is zero.
pub fn expected_collator_index(slot: Slot, config: &AuthConfig) -> u32 {
	((slot / config.slot_duration) % config.collator_set_size) as u32
}
