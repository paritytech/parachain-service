//! Core shared types, host functions, constants, and state helpers for the parachain service.
//!
//! Subsumes `parachain-service-interface`, `jam-state-helpers`, and the `authorizer`
//! module of the former `cumulus` facade crate. Used by `pvf`s, `authorizer`s, the
//! collator, and the node-side JAM tooling.

#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;

pub mod authorization;
pub mod authorizer;
pub mod candidate;
pub mod constants;
pub mod host_call;
pub mod para_info;
pub mod proof;
pub mod refine;
pub mod state_key;
pub mod types;
pub mod upward_message;

/// Host fn wrappers for parachain-service-specific indices (200-203).
/// Only linkable on PolkaVM targets.
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub mod host;

// Flat re-exports callers used to get from jam-state-helpers.
pub use constants::MAX_VALIDATION_CODE_SIZE;
pub use para_info::{para_info_key, storage_key, ParaInfo, Tag};
pub use proof::{verify, ProofError, StateProof};
pub use state_key::{service_request_state_key, service_value_state_key};
pub use upward_message::CodeUpgradePhase;

/// The parachain service's JAM service ID. Network constant; must match genesis registration.
pub const PARACHAIN_SERVICE_ID: u32 = 1337;

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
