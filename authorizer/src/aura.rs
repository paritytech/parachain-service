//! AURA-style collator-set authorizer (design §7.1) — the full §7.1 pipeline with real binary
//! Merkle-proof verification, over whichever signature scheme the verifier blob supplies
//! (D-4 resolved).

use alloc::vec::Vec;

use codec::{Decode, Encode};
use jam_types::{Encode as JamEncode, ServiceId, Slot, WorkPackage};
use parachain_service_interface::types::ParaId;
use primitive_types::H256;

// The service decodes the trace without linking this crate, so it lives in the shared interface
// crate; it is re-exported here because this is where the authorizer's wire types read as one set.
pub use parachain_service_interface::authorization::{AuthTrace, CollatorKey, CollatorSignature};

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

#[derive(Clone, Debug, Encode, Decode)]
pub struct AuthToken {
	/// Proof that the `key` is at the slot-selected leaf index in the collator
	/// set trie committed to by `collator_set_root`.
	pub proof: Vec<H256>,

	/// Key of the collator that authored the work package.
	pub key: CollatorKey,

	/// Signature by the `key` over [`signable_work_package_hash`].
	pub signature: CollatorSignature,
}

/// The one thing about a collator's authorization that is not scheme-blind.
///
/// Keys and signatures are raw 32/64-byte arrays, the trie hashes raw key bytes and the signing
/// payload is a hash, so a scheme is exactly this one function. Which scheme a core accepts is
/// settled by the authorizer hash sitting in its queue — the hash commits to the verifier blob's
/// code, and there is one blob per scheme.
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

	/// Verify the collator's signature over the token-free package hash, under `S`.
	///
	/// The hash is the whole of what a collator signs. Everything a package says — its items, its
	/// authorizer, its context — is already inside it; only the token is not, which is what lets
	/// the signature sit inside the token.
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

		Ok(AuthTrace { author_key: self.key })
	}
}

/// Number of sibling hashes a proof carries for a set of `collator_set_size` collators:
/// ⌈log₂(size)⌉, the depth of the zero-padded power-of-two tree.
fn proof_depth(collator_set_size: u32) -> usize {
	(u32::BITS - collator_set_size.saturating_sub(1).leading_zeros()) as usize
}

fn blake2b_32(input: &[u8]) -> [u8; 32] {
	let mut out = [0u8; 32];
	out.copy_from_slice(blake2b_simd::Params::new().hash_length(32).hash(input).as_bytes());
	out
}

/// Hash of one collator-set leaf.
fn collator_leaf_hash(key: &CollatorKey) -> [u8; 32] {
	blake2b_32(key)
}

/// Hash of an ordered pair of nodes.
fn join(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
	let mut input = [0u8; 64];
	input[..32].copy_from_slice(left);
	input[32..].copy_from_slice(right);
	blake2b_32(&input)
}

/// Build the collator-set trie [`AuthToken::check_proof`] verifies against, returning its root
/// and one proof per collator, in set order.
///
/// Lives next to `check_proof` on purpose: it is that function's inverse, and everything the
/// scheme leaves open — leaf hashing, node hashing, sibling order, padding — is pinned once, in
/// the doc comment there, by code both sides share. The collator and the tests all build their
/// roots here, so a change to the protocol cannot move one side without the other.
///
/// Panics on an empty set: a collator set nobody is in authorizes nothing.
pub fn build_collator_tree(keys: &[CollatorKey]) -> (H256, Vec<Vec<H256>>) {
	assert!(!keys.is_empty(), "a collator set must have at least one collator");

	let mut level: Vec<[u8; 32]> = keys.iter().map(collator_leaf_hash).collect();
	level.resize(keys.len().next_power_of_two(), [0u8; 32]);

	// Every level except the root; a proof takes one sibling from each of them.
	let mut levels = Vec::new();
	while level.len() > 1 {
		let parents = level.chunks(2).map(|pair| join(&pair[0], &pair[1])).collect();
		levels.push(level);
		level = parents;
	}

	let proofs = (0..keys.len())
		.map(|leaf| {
			levels
				.iter()
				.enumerate()
				.map(|(depth, nodes)| H256::from(nodes[(leaf >> depth) ^ 1]))
				.collect()
		})
		.collect();

	(H256::from(level[0]), proofs)
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
