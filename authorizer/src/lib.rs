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

use alloc::vec::Vec;

use codec::{Decode, Encode};
use jam_types::{AuthTrace, CoreIndex, Encode as JamEncode, WorkPackage};
pub use parachain_support::types::ParaId;
use primitive_types::H256;

mod is_authorized;

/// Directory of this crate's `Cargo.toml`, used by `parachain-authorizer-bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Domain separator for the token-free work-package hash signed by AURA collators.
const SIGNABLE_WORK_PACKAGE_DOMAIN: &[u8] = b"parachain-service:aura:work-package:v1";

pub type CollatorKey = [u8; 32];
pub type CollatorSignature = [u8; 64];

/// Hash of a work-package that can be signed by AURA collators.
///
/// This excludes the authorization token since that would contain said signature.
pub fn signable_work_package_hash(package: &WorkPackage) -> H256 {
    let mut signable = Vec::new();
    signable.extend_from_slice(SIGNABLE_WORK_PACKAGE_DOMAIN);
    JamEncode::encode_to(
        &(
            &package.auth_code_host,
            &package.authorizer.code_hash,
            &package.context,
            &package.authorizer.config,
            &package.items,
        ),
        &mut signable,
    );

    let hash = blake2b_simd::Params::new().hash_length(32).hash(&signable);
    H256::from_slice(hash.as_bytes())
}

pub struct ParachainAuthorizer;
jam_pvm_common::declare_authorizer!(ParachainAuthorizer);

impl jam_pvm_common::Authorizer for ParachainAuthorizer {
    fn is_authorized(core: CoreIndex) -> AuthTrace {
        is_authorized::is_authorized(core)
    }
}

#[derive(Debug, Encode, Decode)]
pub struct AuraAuthConfig {
    pub para_ids: Vec<ParaId>,
    pub collator_set_root: H256,
    pub collator_set_size: u32,
    /// In multiples of JAM 6 second slots
    pub slot_duration: u32,
}

#[derive(Debug, Encode, Decode)]
pub struct AuraCollatorAuthToken {
    /// Proof that the `key` is in the `collator_set_root` of the Aura auth config.
    pub proof: Vec<H256>,

    /// Key of the collator that authored the work package.
    pub key: CollatorKey,

    /// Signature by the `key` over the work package hash.
    pub signature: CollatorSignature,
}

impl AuraCollatorAuthToken {
    pub fn check_proof(&self, config: &AuraAuthConfig) -> bool {
        // FIXME unmock
        self.proof == config.collator_set_root.as_ref();
        true
    }

    pub fn check_signature(&self, work_package_hash: H256) -> bool {
        // FIXME unmock
        self.signature == work_package_hash.as_ref();
        true
    }

    pub fn try_into_trace(
        &self,
        config: &AuraAuthConfig,
        wp: &WorkPackage,
    ) -> Option<AuraAuthTrace> {
        let wp_hash = signable_work_package_hash(wp);

        let good = self.check_proof(config) && self.check_signature(wp_hash);
        good.then_some(AuraAuthTrace {
            author_key: self.key.clone(),
        })
    }
}

#[derive(Debug, Encode, Decode)]
pub struct AuraAuthTrace {
    pub author_key: CollatorKey,
}
