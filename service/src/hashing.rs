//! Guest-side hashing.
//!
//! JAM's standard hash is blake2b-256 (`jam_std_common::hash_raw` on the host).
//! The service, the PVF's `set_parent_head_hash`, and JAM's own preimage
//! hashing therefore agree.

use jam_types::Hash;
use tiny_keccak::{Hasher as _, Keccak};

/// blake2b-256 of `data`, matching `jam_std_common::hash_raw`.
pub fn blake2_256(data: &[u8]) -> Hash {
	let hash = blake2b_simd::Params::new().hash_length(32).hash(data);
	hash.as_bytes().try_into().expect("hash_length(32) yields 32 bytes; qed")
}

/// keccak-256 of `data`, used by the §5.5 head commitment.
pub fn keccak_256(data: &[u8]) -> Hash {
	let mut keccak = Keccak::v256();
	keccak.update(data);
	let mut out = Hash::default();
	keccak.finalize(&mut out);
	out
}
