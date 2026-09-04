//! AURA-style collator-set authorizer (design §7.1) — full §7.1 pipeline with
//! real binary Merkle-proof and ed25519 `verify_strict` signature verification
//! (D-4 resolved).

use alloc::vec::Vec;

use codec::{Decode, Encode};
use ed25519_dalek::{Signature, VerifyingKey};
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
	/// coretime being spent on other JAM work.
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
	/// Whether the package was admitted through a privileged control lane.
	/// This branch has no such lane and always emits `false`; the field keeps
	/// the wire shape aligned with the deployed sr25519 authorizer.
	pub sudo: bool,
}

/// Authorization token validation failed.
#[derive(Debug)]
pub enum TokenError {
	BadCollatorSetProof,
	BadCollatorSignature,
}

impl AuthToken {
	/// Verify that `key` sits at leaf `collator_index` of the collator-set trie.
	///
	/// Protocol pinned here (the spec leaves the hash function and bit order
	/// undefined):
	///
	/// - **Leaf hash**: blake2b-32 over the raw 32-byte key.
	/// - **Node hash**: blake2b-32 over the concatenated left–right pair.
	/// - **Sibling ordering**: LSB-first from `collator_index`; bit = 0 means the current node is
	///   the left child (proof sibling is right), bit = 1 means the current node is the right child
	///   (proof sibling is left).
	/// - **Padding**: tree is zero-hash-padded to the next power of two.
	/// - **Proof length**: ⌈log₂(collator_set_size)⌉.
	///
	/// A wrong proof length or a mismatched root → `TokenError::BadCollatorSetProof`.
	pub fn check_proof(&self, config: &AuthConfig, collator_index: u32) -> Result<(), TokenError> {
		let n = config.collator_set_size;
		let expected_depth = (u32::BITS - n.saturating_sub(1).leading_zeros()) as usize;
		if self.proof.len() != expected_depth {
			return Err(TokenError::BadCollatorSetProof);
		}

		let mut current = [0u8; 32];
		current.copy_from_slice(
			blake2b_simd::Params::new().hash_length(32).hash(self.key.as_ref()).as_bytes(),
		);

		for (level, sibling) in self.proof.iter().enumerate() {
			let bit = (collator_index >> level) & 1;
			let mut input = [0u8; 64];
			if bit == 0 {
				input[..32].copy_from_slice(&current);
				input[32..].copy_from_slice(sibling.as_bytes());
			} else {
				input[..32].copy_from_slice(sibling.as_bytes());
				input[32..].copy_from_slice(&current);
			}
			current.copy_from_slice(
				blake2b_simd::Params::new().hash_length(32).hash(&input).as_bytes(),
			);
		}

		if H256::from_slice(&current) == config.collator_set_root {
			Ok(())
		} else {
			Err(TokenError::BadCollatorSetProof)
		}
	}

	/// Verify the collator's ed25519 signature over the token-free package hash.
	///
	/// Uses `verify_strict` (not `verify`) to reject cofactored/non-canonical
	/// signatures and low-order public keys — required for deterministic
	/// validator agreement across implementations.
	pub fn check_signature(&self, work_package_hash: H256) -> Result<(), TokenError> {
		let vk =
			VerifyingKey::from_bytes(&self.key).map_err(|_| TokenError::BadCollatorSignature)?;
		let sig = Signature::from_bytes(&self.signature);
		vk.verify_strict(work_package_hash.as_bytes(), &sig)
			.map_err(|_| TokenError::BadCollatorSignature)
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

		Ok(AuthTrace { author_key: self.key, sudo: false })
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
