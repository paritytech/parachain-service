use alloc::vec::Vec;
use codec::{Decode, Encode};
use jam_types::WorkPackage;
use primitive_types::H256;

use crate::{
	merkle::{collator_leaf_hash, join, proof_depth},
	signable_work_package_hash, AuthConfig, AuthTrace, CollatorKey, CollatorSignature,
};

#[derive(Clone, Debug, Encode, Decode)]
pub struct AuthToken {
	/// Proof that the `key` is at the slot-selected leaf index in the collator
	/// set tree committed to by `collator_set_root`.
	pub proof: Vec<H256>,

	/// Key of the collator that authored the work package, or the local development authorizer
	/// sentinel.
	pub key: CollatorKey,

	/// Signature by the `key` over [`signable_work_package_hash`].
	pub signature: CollatorSignature,
}

/// Signature verification supplied by the verifier program.
///
/// The authorizer code hash selects the scheme; keys and signatures have the same wire format.
pub trait SignatureScheme {
	/// Whether `signature` is `key`'s signature over `payload`.
	fn verify(key: &CollatorKey, signature: &CollatorSignature, payload: &[u8]) -> bool;
}

/// Authorization token validation failed.
#[derive(Debug)]
pub enum TokenError {
	BadCollatorSetProof,
	BadCollatorSignature,
}

impl AuthToken {
	/// Verify that `key` sits at leaf `collator_index` of the collator-set tree.
	///
	/// Proof format:
	///
	/// - **Leaf hash**: blake2b-32 over the raw 32-byte key.
	/// - **Node hash**: blake2b-32 over the concatenated left–right pair.
	/// - **Sibling ordering**: LSB-first from `collator_index`; bit = 0 means the current node is
	///   the left child (proof sibling is right), bit = 1 means the current node is the right child
	///   (proof sibling is left).
	/// - **Padding**: tree is zero-hash-padded to the next power of two.
	/// - **Proof length**: ⌈log₂(collator_set_size)⌉.
	///
	/// Returns [`TokenError::BadCollatorSetProof`] for a wrong proof length or root.
	/// The caller must provide a nonzero set size and an in-range `collator_index`.
	pub fn check_proof(&self, config: &AuthConfig, collator_index: u32) -> Result<(), TokenError> {
		if self.proof.len() != proof_depth(config.collator_set_size) {
			return Err(TokenError::BadCollatorSetProof);
		}

		let mut current = collator_leaf_hash(&self.key);
		for (level, sibling) in self.proof.iter().enumerate() {
			let bit = (collator_index >> level) & 1;
			current = if bit == 0 {
				join(&current, &sibling.to_fixed_bytes())
			} else {
				join(&sibling.to_fixed_bytes(), &current)
			};
		}

		if H256::from(current) == config.collator_set_root {
			Ok(())
		} else {
			Err(TokenError::BadCollatorSetProof)
		}
	}

	/// Verify the signature over [`signable_work_package_hash`] using `S`.
	pub fn check_signature<S: SignatureScheme>(
		&self,
		work_package_hash: H256,
	) -> Result<(), TokenError> {
		S::verify(&self.key, &self.signature, work_package_hash.as_bytes())
			.then_some(())
			.ok_or(TokenError::BadCollatorSignature)
	}

	/// Run the §7.1 token checks for the slot-selected `collator_index` and
	/// produce the trace carrying the author key.
	pub fn try_into_trace<S: SignatureScheme>(
		&self,
		config: &AuthConfig,
		wp: &WorkPackage,
		collator_index: u32,
	) -> Result<AuthTrace, TokenError> {
		let wp_hash = signable_work_package_hash(wp);

		self.check_proof(config, collator_index)?;
		self.check_signature::<S>(wp_hash)?;

		Ok(AuthTrace { author_key: self.key, sudo: false })
	}
}
