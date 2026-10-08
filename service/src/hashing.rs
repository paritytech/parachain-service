//! Guest-side hashing.
//!
//! JAM's standard hash is blake2b-256 (`jam_std_common::hash_raw` on the host).
//! The service, the PVF's `set_parent_head_hash`, and JAM's own preimage
//! hashing therefore agree.
//!
//! §5.5's head commitment is the exception: its tree — leaves included — is
//! keccak-256, as Ethereum specifies it. So `head_data` is digested under *two*
//! functions, blake2b-256 for the §5.1 parent-head check and keccak-256 for the
//! leaf a commitment carries. They are deliberately different domains: a
//! commitment leaf must never verify against a candidate's declared parent head.

pub use sp_crypto_hashing::{blake2_256, keccak_256};
