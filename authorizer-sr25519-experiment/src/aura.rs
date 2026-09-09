//! EXPERIMENT copy of `authorizer/src/aura.rs` with the ed25519 signature check
//! swapped for sr25519 (schnorrkel). Nothing else differs — keep it that way, the
//! measurement in `../README.md` is only worth something if it stays a one-line diff.

use alloc::vec::Vec;

use codec::{Decode, Encode};
use jam_types::{Encode as JamEncode, ServiceId, Slot, WorkPackage};
use parachain_service_interface::types::ParaId;
use primitive_types::H256;
use schnorrkel::{PublicKey, Signature};

// The service decodes the trace (and so the command in it) without linking this crate, so both
// live in the shared interface crate; they are re-exported here because this is where the
// authorizer's wire types read as one set.
pub use parachain_service_interface::authorization::{
	AuthTrace, CollatorKey, CollatorSignature, Command,
};

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

	/// Signature by the `key` over [`AuthToken::signing_payload`].
	pub signature: CollatorSignature,

	/// A core-assignment command for the Parachain Service, normally `None`.
	///
	/// The authorizer only echoes it into the trace once the token checks out; executing it is
	/// the service's business, in Accumulate.
	pub control_command: Option<Command>,
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
	/// Protocol pinned here (SPEC_GAPS #7 — spec leaves hash function and bit
	/// order undefined):
	///
	/// - **Leaf hash**: blake2b-32 over the raw 32-byte key.
	/// - **Node hash**: blake2b-32 over the concatenated left–right pair.
	/// - **Sibling ordering**: LSB-first from `collator_index`; bit = 0 means the
	///   current node is the left child (proof sibling is right), bit = 1 means
	///   the current node is the right child (proof sibling is left).
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

	/// Verify the collator's sr25519 signature over the token-free package hash.
	///
	/// The shipping crate uses ed25519 `verify_strict` here, to reject cofactored
	/// or non-canonical signatures and low-order public keys. There is no
	/// equivalent knob on this side and none is needed: ristretto is a
	/// prime-order group with a canonical encoding, so `PublicKey::from_bytes`
	/// already rejects what `verify_strict` exists to reject.
	pub fn check_signature(&self, work_package_hash: H256) -> Result<(), TokenError> {
		let payload = Self::signing_payload(work_package_hash, &self.control_command);
		let public =
			PublicKey::from_bytes(&self.key).map_err(|_| TokenError::BadCollatorSignature)?;
		let signature =
			Signature::from_bytes(&self.signature).map_err(|_| TokenError::BadCollatorSignature)?;
		public
			.verify_simple(SIGNING_CONTEXT, payload.as_bytes(), &signature)
			.map_err(|_| TokenError::BadCollatorSignature)
	}

	/// What a collator actually signs: the token-free package hash bound to the control command
	/// the token carries.
	///
	/// The command cannot travel in the package hash, because it lives in the token and the
	/// package hash excludes the token by construction (that is what lets the signature sit
	/// inside it). Binding it here is what stops a command being bolted onto somebody else's
	/// package while it is in flight. Signers must go through this function, `None` included.
	pub fn signing_payload(work_package_hash: H256, control_command: &Option<Command>) -> H256 {
		H256::from(blake2b_32(&(work_package_hash, control_command).encode()))
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

		Ok(AuthTrace { author_key: self.key, control_command: self.control_command.clone() })
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
/// the doc comment there, by code both sides share. The collator, `parasim-tool` and the tests
/// all build their roots here, so a change to the protocol cannot move one side without the
/// other.
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

/// sr25519 signing-context domain separator.
pub const SIGNING_CONTEXT: &[u8] = b"jam:parachain-service:aura";

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
