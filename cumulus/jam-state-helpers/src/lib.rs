//! JAM state-key derivation and state-proof verification, usable from a service guest.
//!
//! Refine cannot read JAM state: there is no state-read host call in-core. A service that needs
//! to know something about the state at its work package's anchor must therefore have the value
//! *proved* to it — the proof travels with the work and is checked against
//! `RefineContext::state_root`, which JAM validates on-chain when the package is reported.
//!
//! This crate is the verifier half of that arrangement. It is deliberately `no_std` and depends
//! on nothing host-only, so the same code runs in the PVM guest and in the collator-side tooling
//! that fetches the proofs — one implementation, so the two cannot disagree.
//!
//! [`jam_std_common`](https://docs.rs/jam-std-common) already provides both halves on the host,
//! but it is std-only (jsonrpsee, futures, ark-vrf), so the guest cannot use it. The derivation
//! here is byte-pinned against it by a test rather than trusted to stay in step by inspection.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod proof;
pub mod state_key;

pub use proof::{verify, ProofError, StateProof};
pub use state_key::service_value_state_key;

/// A JAM state key: 31 octets, per the Gray Paper's state-merklization appendix.
pub type StateKey = [u8; 31];

/// A trie node as it appears in a proof: 512 bits, per the Gray Paper.
pub type ProofNode = [u8; 64];

/// A 32-octet blake2b hash — a node hash, a state root, or a hashed value.
pub type Hash = [u8; 32];

/// blake2b-256, JAM's standard hash (`jam_std_common::hash_raw`).
pub fn blake2_256(data: &[u8]) -> Hash {
	let hash = blake2b_simd::Params::new().hash_length(32).hash(data);
	hash.as_bytes().try_into().expect("hash_length(32) yields 32 bytes; qed")
}
