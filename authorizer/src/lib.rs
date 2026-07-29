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

use jam_pvm_common::{info, is_authorized::auth_token};
use jam_types::{AuthTrace, Authorization, CoreIndex};

pub struct ParachainAuthorizer;
jam_pvm_common::declare_authorizer!(ParachainAuthorizer);

impl jam_pvm_common::Authorizer for ParachainAuthorizer {
    fn is_authorized(core: CoreIndex) -> AuthTrace {
        // The token is the package-supplied input to the authorizer (from
        // `WorkPackage::authorization`); the fixed per-authorizer parameter lives
        // in `Authorizer::config` (fetch via `is_authorized::auth_config()`).
        let Authorization(token) = auth_token();
        info!("is_authorized on core {core}: {} token bytes", token.len());

        // --- FILL IN: the real authorization policy. ---
        // Reject by panicking (`assert!`/`panic!`) when the package is not
        // authorized for this core. For now we authorize unconditionally and
        // forward the token as the auth trace so refine/accumulate can see it.
        AuthTrace(token)
    }
}
