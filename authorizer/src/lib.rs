#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! JAM host adapter for AURA authorization.
//!
//! Shared protocol types live in `cumulus-aura-authorizer-utils`. The ed25519 and sr25519 verifier
//! programs supply signature verification and call [`authorize`] from their entry points.

extern crate alloc;

use alloc::{format, vec::Vec};

use codec::Encode;
pub use cumulus_aura_authorizer_utils::ParaId;
use jam_types::{AuthTrace, CoreIndex};

pub use cumulus_aura_authorizer_utils as aura;
pub mod block;
mod is_authorized;

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

/// Run block authorization with the signature scheme selected by the verifier program.
///
/// Returns the encoded [`aura::AuthTrace`] and panics if the block is not authorized.
pub fn authorize_block<S: aura::SignatureVerifier>(input: &[u8]) -> Vec<u8> {
	match block::authorize_block::<S>(input) {
		Ok(trace) => trace.encode(),
		Err(e) => {
			let msg = format!("Block is not authorized: {e:?}");

			jam_pvm_common::error!("{msg}");
			panic!("{msg}");
		},
	}
}
