//! Core shared wire types, constants, and storage key helpers for the parachain service.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod constants;
pub mod para_info;
pub mod state_key;
pub mod types;

pub use constants::MAX_VALIDATION_CODE_SIZE;
pub use para_info::{para_info_key, para_log_key, storage_key, ParaInfo, Tag};
pub use state_key::{
	service_preimage_state_key, service_request_state_key, service_value_state_key,
};

/// The parachain service's JAM service ID. Network constant; must match genesis registration.
pub const PARACHAIN_SERVICE_ID: types::ServiceId = 1337;

/// A JAM state key: 31 octets, per the Gray Paper's state-merklization appendix.
pub type StateKey = [u8; 31];

/// A 32-octet blake2b hash — a node hash, a state root, or a hashed value.
pub type Hash = [u8; 32];

/// blake2b-256, JAM's standard hash (`jam_std_common::hash_raw`).
pub use sp_crypto_hashing::blake2_256;
