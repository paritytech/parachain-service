#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! Authorizer PVM module for the parachain service.
//!
//! On JAM the authorizer is a **separate** program from the service: it builds
//! into its own blob (`target/parachain-authorizer.jam`) with its own code hash
//! and is referenced by the work package's `authorizer.code_hash`, distinct from
//! the work item's service `code_hash`. Its single entry point, `is_authorized`,
//! decides whether a work package may run on a given core and returns an opaque
//! "auth trace" that is handed to both refine and accumulate for every work item
//! in the package. Our service echoes it back via `refine::auth_trace()`.

extern crate alloc;

use alloc::format;

use jam_types::{AuthTrace, CoreIndex};
pub use parachain_support::types::ParaId;

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
