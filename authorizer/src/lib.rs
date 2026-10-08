#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! JAM host adapter for AURA authorization.
//!
//! Shared protocol types live in `jp-aura-authorizer`. The ed25519 and sr25519 verifier
//! programs supply signature verification and call [`authorize`] from their entry points.

extern crate alloc;

use alloc::format;

use jam_types::{AuthTrace, CoreIndex};
pub use jp_aura_authorizer::ParaId;

pub mod aura;
pub mod is_authorized;

/// Run authorization with the signature scheme selected by the verifier program.
pub fn authorize<S: aura::SignatureVerifier>(core: CoreIndex) -> AuthTrace {
	match is_authorized::is_authorized::<S>(core) {
		Ok(r) => r,
		Err(e) => {
			let msg = format!("BUG: Parachain Service is_authorized crashed: {e:?}");

			jam_pvm_common::error!("{msg}");
			panic!("{msg}");
		},
	}
}
