//! Guest-side hashing.
//!
//! JAM's standard hash is blake2b-256 (`jam_std_common::hash_raw` on the host).
//! The design doc's `hash(head_data)` in §5.1 step 3 does not name a function;
//! we pin blake2b-256 so the service, the PVF's `set_parent_head_hash`, and
//! JAM's own preimage hashing all agree. TODO: needs upstreaming into the spec.

use jam_types::Hash;

/// blake2b-256 of `data`, matching `jam_std_common::hash_raw`.
pub fn blake2_256(data: &[u8]) -> Hash {
	let hash = blake2b_simd::Params::new().hash_length(32).hash(data);
	hash.as_bytes().try_into().expect("hash_length(32) yields 32 bytes; qed")
}
