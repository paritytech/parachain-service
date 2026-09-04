#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! Scheme-blind core of the parachain service's authorizer.
//!
//! On JAM the authorizer is a **separate** program from the service: it builds into its own blob
//! with its own code hash and is referenced by the work package's `authorizer.code_hash`, distinct
//! from the work item's service `code_hash`. Its single entry point, `is_authorized`, decides
//! whether a work package may run on a given core and returns an opaque "auth trace" that is
//! handed to both refine and accumulate for every work item in the package. Our service echoes it
//! back via `refine::auth_trace()`.
//!
//! This crate is not a blob. Everything here — the config, the token, the collator trie, the
//! signing payload, the round-robin — is the same whatever curve the collators sign on; the one
//! function that is not is [`aura::SignatureScheme`]. The blobs are
//! `parachain-authorizer-ed25519` and `parachain-authorizer-sr25519`, each of which is that one
//! function plus [`authorize`].

extern crate alloc;

use alloc::format;

use jam_types::{AuthTrace, CoreIndex};
pub use parachain_service_interface::types::ParaId;

pub mod aura;
pub mod is_authorized;

/// The whole of a verifier blob's entry point, bar the scheme it is built for.
pub fn authorize<S: aura::SignatureScheme>(core: CoreIndex) -> AuthTrace {
	match is_authorized::is_authorized::<S>(core) {
		Ok(r) => r,
		Err(e) => {
			let msg = format!("BUG: Parachain Service is_authorized crashed: {e:?}");

			jam_pvm_common::error!("{msg}");
			panic!("{msg}");
		},
	}
}
