//! Core shared wire types, constants, and storage key helpers for the parachain service.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod constants;
pub mod para_info;
pub mod state_key;
pub mod types;

pub use constants::MAX_VALIDATION_CODE_SIZE;
pub use para_info::{ParaInfo, StorageKey};
pub use state_key::{
	service_preimage_state_key, service_request_state_key, service_value_state_key,
};

/// The parachain service's JAM service ID.
pub const PARACHAIN_SERVICE_ID: types::ServiceId = 1337;

/// A 31-byte JAM state key.
pub type StateKey = [u8; 31];

/// A 32-byte blake2b hash.
pub type Hash = [u8; 32];

/// blake2b-256 hash function.
pub use sp_crypto_hashing::blake2_256;
