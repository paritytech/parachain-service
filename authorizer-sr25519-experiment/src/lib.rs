#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! EXPERIMENT, not part of the product — a copy of the `authorizer` crate with
//! the ed25519 signature check swapped for sr25519 (schnorrkel) and nothing else
//! changed.
//!
//! It answers one question: does an sr25519 AURA authorizer still fit JAM's
//! 64,000-byte `C_maxauthcodesize`? It does, with room to spare — 59,349 bytes
//! against the shipping ed25519 blob's 59,328, of which 19 bytes are just this
//! crate's longer name in the blob metadata. It is also about 12% cheaper in gas.
//! `README.md` has the full numbers, how to re-measure them, and what got in the
//! way.
//!
//! Only `aura::AuthToken::check_signature` differs from the shipping crate.
//!
//! On JAM the authorizer is a **separate** program from the service: it builds
//! into its own blob (`parachain-authorizer-sr25519-experiment.jam`) with its own code hash
//! and is referenced by the work package's `authorizer.code_hash`, distinct from
//! the work item's service `code_hash`. Its single entry point, `is_authorized`,
//! decides whether a work package may run on a given core and returns an opaque
//! "auth trace" that is handed to both refine and accumulate for every work item
//! in the package. Our service echoes it back via `refine::auth_trace()`.

extern crate alloc;

use alloc::format;

/// Bump the PVM guest stack for `curve25519-dalek`'s serial-64 double-scalar
/// multiplication (its Straus tables exceed polkavm's default 8 KiB stack).
/// Unchanged from the shipping crate: schnorrkel's ristretto verify fits in the
/// same 32 KiB.
/// No-op on host builds (the `polkavm_derive` macro is only defined on riscv+e).
macro_rules! min_stack_size {
	($size:expr) => {
		#[cfg(all(any(target_arch = "riscv32", target_arch = "riscv64"), target_feature = "e"))]
		polkavm_derive::min_stack_size!($size);
	};
}
min_stack_size!(32768); // 32 KiB guest stack.

use jam_types::{AuthTrace, CoreIndex};
pub use parachain_service_interface::types::ParaId;

pub mod aura;
pub mod is_authorized;

/// Directory of this crate's `Cargo.toml`, used by `parachain-authorizer-bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

pub struct ParachainAuthorizer;
jam_pvm_common::declare_authorizer!(ParachainAuthorizer);

impl jam_pvm_common::Authorizer for ParachainAuthorizer {
	fn is_authorized(core: CoreIndex) -> AuthTrace {
		match is_authorized::is_authorized(core) {
			Ok(r) => r,
			Err(e) => {
				let msg = format!("BUG: Parachain Service is_authorized crashed: {e:?}");

				jam_pvm_common::error!("{msg}");
				panic!("{msg}");
			},
		}
	}
}
