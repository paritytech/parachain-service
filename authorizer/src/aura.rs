//! AURA-style collator-set authorizer (design §7.1) — demonstration authorizer
//! with the full §7.1 pipeline; signature and Merkle-proof verification are
//! stubbed per DECISIONS.md D-4.

use alloc::vec::Vec;

use codec::{Decode, Encode};
use jam_types::{Encode as JamEncode, ServiceId, Slot, WorkPackage};
use parachain_service_interface::types::ParaId;
use primitive_types::H256;

pub type CollatorKey = [u8; 32];
pub type CollatorSignature = [u8; 64];

#[derive(Debug, Encode, Decode)]
pub struct AuthConfig {
	/// Authoritative `ParaId` for each work item, in item order (§3.2). Must
	/// stay the config's first field: the Parachain Service's Refine decodes
	/// exactly this prefix.
	pub para_ids: Vec<ParaId>,
	/// The JAM service every work item must target. Prevents para-specific
	/// coretime being spent on other JAM work (SPEC_GAPS #7).
	/// TODO: not yet in the design's §7.1 config; needs upstreaming.
	pub parachain_service: ServiceId,
	/// Root of a binary Merkle trie over the collator public keys.
	/// Leaf index == collator index in the set.
	pub collator_set_root: H256,
	/// Number of collators in the set. Zero is rejected.
	pub collator_set_size: u32,
	/// Slot duration as a multiple of the JAM timeslot (6 s). Zero is rejected.
	pub slot_duration: u32,
}

#[derive(Debug, Encode, Decode)]
pub struct AuthToken {
	/// Proof that the `key` is at the slot-selected leaf index in the collator
	/// set trie committed to by `collator_set_root`.
	pub proof: Vec<H256>,

	/// Key of the collator that authored the work package.
	pub key: CollatorKey,

	/// Signature by the `key` over the token-free work package hash.
	pub signature: CollatorSignature,
}

#[derive(Debug, Encode, Decode)]
#[cfg_attr(feature = "test-utils", derive(codec::MaxEncodedLen))]
pub struct AuthTrace {
	pub author_key: CollatorKey,
}

/// Authorization token validation failed.
#[derive(Debug)]
pub enum TokenError {
	BadCollatorSetProof,
	BadCollatorSignature,
}

impl AuthToken {
	/// Verify that `key` sits at leaf `collator_index` of the set trie.
	///
	/// FIXME: stubbed (D-4) — accepts the mock proof `[collator_set_root]`
	/// regardless of `collator_index`. The real check must recompute the binary
	/// Merkle root from `blake2(key)` at `collator_index` along `proof` (with a
	/// specified non-power-of-two padding rule, SPEC_GAPS #7) and compare.
	pub fn check_proof(&self, config: &AuthConfig, _collator_index: u32) -> Result<(), TokenError> {
		if self.proof.as_slice() == &[config.collator_set_root] {
			Ok(())
		} else {
			Err(TokenError::BadCollatorSetProof)
		}
	}

	/// Verify the collator's signature over the token-free package hash.
	///
	/// FIXME: stubbed (D-4) — accepts the mock signature `[255; 64]`. The real
	/// check is an ed25519 verification of `work_package_hash` under `key`.
	pub fn check_signature(&self, _work_package_hash: H256) -> Result<(), TokenError> {
		if self.signature == [255; 64] {
			Ok(())
		} else {
			Err(TokenError::BadCollatorSignature)
		}
	}

	/// Run the §7.1 token checks for the slot-selected `collator_index` and
	/// produce the trace carrying the author key.
	pub fn try_into_trace(
		&self,
		config: &AuthConfig,
		wp: &WorkPackage,
		collator_index: u32,
	) -> Result<AuthTrace, TokenError> {
		let wp_hash = signable_work_package_hash(wp);

		self.check_proof(config, collator_index)?;
		self.check_signature(wp_hash)?;

		Ok(AuthTrace { author_key: self.key })
	}
}

/// §7.1 step 4 — the round-robin collator index expected for `slot`:
/// `(slot / slot_duration) mod collator_set_size`. The caller rejects zero
/// `slot_duration` / `collator_set_size` beforehand.
pub fn expected_collator_index(slot: Slot, config: &AuthConfig) -> u32 {
	((slot / config.slot_duration) % config.collator_set_size) as u32
}

/// Domain separator for the token-free work-package hash signed by AURA collators.
pub const WORK_PACKAGE_SIGN_CTX: &[u8] = b"jam:parachain-service:aura:work-package:v1";

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
