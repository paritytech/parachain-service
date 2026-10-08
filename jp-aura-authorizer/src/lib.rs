//! Shared AURA authorization types, signing payloads, and collator proofs.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod authorization;
mod config;
mod merkle;
mod signing;
mod token;

pub use polkadot_parachain_primitives::primitives::Id as ParaId;

pub use authorization::{AuthTrace, CollatorKey, CollatorSignature};
pub use config::{expected_collator_index, AuthConfig};
pub use merkle::build_collator_tree;
pub use signing::{signable_work_package_hash, WORK_PACKAGE_SIGN_CTX};
pub use token::{AuthToken, SignatureScheme, TokenError};
