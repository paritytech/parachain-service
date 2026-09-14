//! Verifying a JAM state proof for a single key against a trusted state root.
//!
//! JAM's state trie is the Gray Paper's *binary* trie (merklization appendix): keys are 31 octets
//! walked one bit at a time from the most significant, and every node is exactly 512 bits:
//!
//! | first bits | node        | octets 0..32                          | octets 32..64            |
//! |------------|-------------|---------------------------------------|--------------------------|
//! | `0`        | branch      | left child hash, MSB replaced by `0`  | right child hash         |
//! | `10`       | small leaf  | `0x80 \| len`, then the 31-octet key   | the value, zero-padded   |
//! | `11`       | large leaf  | `0xc0`, then the 31-octet key          | blake2b-256 of the value |
//!
//! A node's identity is `blake2b_256` of its 64 octets, and an empty (sub-)trie is the zero hash.
//!
//! Because the branch discriminator overwrites the top bit of the *left* child hash, only 255 of
//! its bits survive on the wire, so child identities are compared with that bit masked off — as
//! polkajam's own `Trie::restore_proof` does.
//!
//! # Why a path walk rather than a trie rebuild
//!
//! polkajam verifies a *range* proof by rebuilding a partial trie and recomputing its root
//! (`vendor/polkajam/crates/trie/src/trie.rs`, `verify_range_proof`). We only ever prove one key,
//! and for a single key the same guarantee falls out of walking the authenticated path: start at
//! the trusted root, and at each step require that the supplied node actually hashes to the
//! identity its parent committed to. Nothing unauthenticated is ever consulted, and there is no
//! partial-trie machinery to get subtly wrong.
//!
//! Absence is a first-class outcome, not an error: the walk proves it by reaching either an empty
//! child slot or a leaf holding a different key. That is how the very first parachain block is
//! recognised — nothing has been stored for the para yet.

use alloc::{collections::BTreeMap, vec::Vec};
use codec::{Decode, Encode};

use crate::{blake2_256, Hash, ProofNode, StateKey};

/// Longest possible path: one bit per bit of a 31-octet key.
const MAX_DEPTH: usize = 31 * 8;

/// An empty (sub-)trie.
const EMPTY_HASH: Hash = [0u8; 32];

/// Largest value a leaf can hold inline.
const MAX_EMBEDDED_VALUE_LEN: usize = 32;

/// A state proof for one key, as carried alongside a work package.
///
/// Deliberately its own type rather than polkajam's `RangeProof`: that one is a host-side,
/// JSON/base64 type with no SCALE codec at all, whereas this travels inside a SCALE-encoded
/// payload. Host-side tooling converts at the edge.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct StateProof {
	/// The nodes along the proved path, in any order. Extra nodes are harmless.
	pub nodes: Vec<ProofNode>,
	/// Preimages for values too large to sit inside their leaf.
	pub values: Vec<(StateKey, Vec<u8>)>,
}

/// Why a state proof could not be trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ProofError {
	/// A node needed to continue the walk was not supplied, so nothing can be concluded about
	/// the key — neither presence nor absence.
	IncompleteProof,
	/// A supplied node is not a well-formed trie node.
	MalformedNode,
	/// A large leaf's value preimage was missing, or did not hash to the leaf's commitment.
	ValueMismatch,
	/// The path was longer than a 31-octet key can address, so the proof cannot be from this trie.
	PathTooLong,
}

/// The value stored under `key` in the state whose root is `state_root`, or `None` if the proof
/// shows that `key` is absent.
///
/// `state_root` must come from a source the caller already trusts — in a service guest, that is
/// `RefineContext::state_root`, which JAM checks on-chain when the work package is reported.
pub fn verify(
	proof: &StateProof,
	state_root: &Hash,
	key: &StateKey,
) -> Result<Option<Vec<u8>>, ProofError> {
	if state_root == &EMPTY_HASH {
		// An empty trie holds nothing, and has no root node for the walk to start from.
		return Ok(None);
	}

	let nodes: BTreeMap<Hash, &ProofNode> =
		proof.nodes.iter().map(|node| (masked(&blake2_256(node)), node)).collect();

	let mut expected = *state_root;
	for depth in 0..MAX_DEPTH {
		let node = nodes.get(&masked(&expected)).ok_or(ProofError::IncompleteProof)?;

		match classify(node)? {
			Node::Branch { children } => {
				let child = children[bit_at(key, depth) as usize];
				if child == EMPTY_HASH {
					// The subtree our key belongs to is empty: the key cannot be there.
					return Ok(None);
				}
				expected = child;
			},
			Node::Leaf { leaf_key, value } => {
				if leaf_key != *key {
					// The only leaf reachable along our key's path holds a different key, so our
					// key is not in the trie.
					return Ok(None);
				}
				return match value {
					LeafValue::Embedded(value) => Ok(Some(value.to_vec())),
					LeafValue::Hashed(hash) => large_value(proof, key, &hash).map(Some),
				};
			},
		}
	}

	Err(ProofError::PathTooLong)
}

/// The `depth`-th bit of `key`, counting from the most significant bit of its first octet.
fn bit_at(key: &StateKey, depth: usize) -> u8 {
	(key[depth / 8] >> (7 - (depth % 8))) & 1
}

/// Clear the bit the branch discriminator overwrites, so that child identities can be compared.
fn masked(hash: &Hash) -> Hash {
	let mut masked = *hash;
	masked[0] &= 0b0111_1111;
	masked
}

enum Node<'a> {
	Branch { children: [Hash; 2] },
	Leaf { leaf_key: StateKey, value: LeafValue<'a> },
}

enum LeafValue<'a> {
	Embedded(&'a [u8]),
	Hashed(Hash),
}

fn classify(node: &ProofNode) -> Result<Node<'_>, ProofError> {
	let (head, tail) = node.split_at(32);

	if head[0] & 0b1000_0000 == 0 {
		let mut left: Hash = head.try_into().expect("split at 32; qed");
		// The discriminator we just read sits where the left child's top bit would be.
		left[0] &= 0b0111_1111;
		let right: Hash = tail.try_into().expect("64 - 32 == 32; qed");
		return Ok(Node::Branch { children: [left, right] });
	}

	let leaf_key: StateKey = head[1..].try_into().expect("32 - 1 == 31; qed");
	let value = if head[0] & 0b0100_0000 == 0 {
		let len = usize::from(head[0] & 0b0011_1111);
		if len > MAX_EMBEDDED_VALUE_LEN {
			return Err(ProofError::MalformedNode);
		}
		LeafValue::Embedded(&tail[..len])
	} else {
		LeafValue::Hashed(tail.try_into().expect("64 - 32 == 32; qed"))
	};
	Ok(Node::Leaf { leaf_key, value })
}

/// The preimage of a large leaf's committed hash, checked against that commitment.
fn large_value(proof: &StateProof, key: &StateKey, hash: &Hash) -> Result<Vec<u8>, ProofError> {
	let value = proof
		.values
		.iter()
		.find_map(|(value_key, value)| (value_key == key).then_some(value))
		.ok_or(ProofError::ValueMismatch)?;

	if blake2_256(value) != *hash {
		return Err(ProofError::ValueMismatch);
	}
	Ok(value.clone())
}
