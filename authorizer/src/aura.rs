use alloc::vec::Vec;

use codec::{Decode, Encode};
use jam_types::{Encode as JamEncode, WorkPackage};
use parachain_support::types::ParaId;
use primitive_types::H256;

pub type CollatorKey = [u8; 32];
pub type CollatorSignature = [u8; 64];

#[derive(Debug, Encode, Decode)]
pub struct AuthConfig {
    pub para_ids: Vec<ParaId>,
    pub collator_set_root: H256,
    pub collator_set_size: u32,
    /// In multiples of JAM 6 second slots
    pub slot_duration: u32,
}

#[derive(Debug, Encode, Decode)]
pub struct AuthToken {
    /// Proof that the `key` is in the `collator_set_root` of the Aura auth config.
    pub proof: Vec<H256>,

    /// Key of the collator that authored the work package.
    pub key: CollatorKey,

    /// Signature by the `key` over the work package hash.
    pub signature: CollatorSignature,
}

#[derive(Debug, Encode, Decode)]
pub struct AuthTrace {
    pub author_key: CollatorKey,
}

/// Authorization token validation failed.
#[derive(Debug)]
pub enum AuthorizationError {
    BadCollatorSetProof,
    BadCollatorSignature,
}

impl AuthToken {
    pub fn check_proof(&self, config: &AuthConfig) -> Result<(), AuthorizationError> {
        // FIXME unmock
        if self.proof.as_slice() == &[config.collator_set_root] {
            Ok(())
        } else {
            Err(AuthorizationError::BadCollatorSetProof)
        }
    }

    pub fn check_signature(&self, work_package_hash: H256) -> Result<(), AuthorizationError> {
        // FIXME unmock
        if self.signature == [255; 64] {
            Ok(())
        } else {
            Err(AuthorizationError::BadCollatorSignature)
        }
    }

    pub fn try_into_trace(
        &self,
        config: &AuthConfig,
        wp: &WorkPackage,
    ) -> Result<AuthTrace, AuthorizationError> {
        let wp_hash = signable_work_package_hash(wp);

        self.check_proof(config)?;
        self.check_signature(wp_hash)?;

        Ok(AuthTrace {
            author_key: self.key.clone(),
        })
    }
}

/// Domain separator for the token-free work-package hash signed by AURA collators.
const WORK_PACKAGE_SIGN_CTX: &[u8] = b"jam:parachain-service:aura:work-package:v1";

/// Hash of a work-package that can be signed by AURA collators.
///
/// This excludes the authorization token since that would contain said signature.
pub fn signable_work_package_hash(package: &WorkPackage) -> H256 {
    let mut signable = Vec::new();
    signable.extend_from_slice(WORK_PACKAGE_SIGN_CTX);
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
