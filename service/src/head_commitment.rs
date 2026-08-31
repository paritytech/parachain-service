//! Parachain head commitment (spec §5.5).
//!
//! `accumulate` returns a commitment to the parachain heads that changed during
//! the block: a binary Merkle tree over one leaf per changed head, ordered by
//! ascending `para_id`, with the root returned. No head changed means no hash.

use crate::{
	hashing::{blake2_256, keccak_256},
	state::para_info::Parachains,
};
use alloc::vec::Vec;
use codec::Encode;
use jam_types::Hash;
use parachain_service_interface::types::ParaId;

/// An element of the commitment tree (§5.5).
///
/// Element hashes are keccak-256 over the SCALE encoding, so the variant
/// discriminant is covered and a leaf hash can never collide with a node hash.
/// A `Leaf` encodes to 37 octets (discriminant, 4-octet `para_id`, 32-octet
/// `head_hash`) and a `Node` to 65 (discriminant, two hashes).
#[derive(Encode)]
enum MerkleTree {
	Node(Hash, Hash),
	Leaf { para_id: ParaId, head_hash: Hash },
}

impl MerkleTree {
	fn hash(&self) -> Hash {
		keccak_256(&self.encode())
	}
}

/// Collapse one level: hash adjacent pairs, promoting a trailing odd element to
/// the next level unchanged rather than duplicating it (D-12).
fn pair_up(level: &[Hash]) -> Vec<Hash> {
	let mut chunks = level.chunks_exact(2);
	let mut out: Vec<Hash> =
		chunks.by_ref().map(|pair| MerkleTree::Node(pair[0], pair[1]).hash()).collect();
	if let [odd] = chunks.remainder() {
		out.push(*odd);
	}
	out
}

/// Root over `leaves`: no leaves yields no hash at all, and a single leaf is its
/// own root rather than being hashed again (§5.5).
fn merkle_root(leaves: Vec<Hash>) -> Option<Hash> {
	let mut level = leaves;
	if level.is_empty() {
		return None;
	}
	while level.len() > 1 {
		level = pair_up(&level);
	}
	level.first().copied()
}

fn head_hash_of(para_id: ParaId) -> Option<Hash> {
	Parachains::get(para_id).map(|pi| blake2_256(&pi.head_data))
}

fn current_head_hashes_of(para_id: ParaId) -> Option<(Hash, Hash)> {
	Parachains::get(para_id).map(|pi| (blake2_256(&pi.head_data), keccak_256(&pi.head_data)))
}

/// Tracks the parachains whose head this block may have moved, so the §5.5
/// commitment can be taken as a diff against their pre-block values.
#[derive(Default)]
pub struct HeadTracker {
	/// Pre-block head hash per touched para; `None` when it did not yet exist.
	prior: Vec<(ParaId, Option<Hash>)>,
}

impl HeadTracker {
	pub fn new() -> Self {
		Self::default()
	}

	/// Snapshot `para_id`'s pre-block head, before its first mutation this block.
	///
	/// Later touches are no-ops, so a head written repeatedly — by a candidate and
	/// then a forced `parachain_set_head`, or across successive invocations — still
	/// yields the single leaf §5.5 requires, carrying the value the block ended
	/// with. Must be called *before* the write it accounts for.
	pub fn touch(&mut self, para_id: ParaId) {
		if self.prior.iter().any(|(p, _)| *p == para_id) {
			return;
		}
		self.prior.push((para_id, head_hash_of(para_id)));
	}

	/// The block's head commitment (§5.5): the Merkle root over the heads that
	/// actually changed, or `None` when none did.
	pub fn commitment(self) -> Option<Hash> {
		let touched = self
			.prior
			.into_iter()
			.map(|(para_id, prior)| (para_id, prior, current_head_hashes_of(para_id)))
			.collect();
		merkle_root(changed_leaves(touched))
	}
}

/// Leaves for the block: one per para whose head actually changed, ordered by
/// ascending `para_id` so every verifier builds the same tree (§5.5).
///
/// A para touched but left on its original value contributes nothing, as does one
/// removed by `parachain_clean_up` — it has no ending value. A newly registered
/// para counts as changed, since it had no prior head.
fn changed_leaves(mut touched: Vec<(ParaId, Option<Hash>, Option<(Hash, Hash)>)>) -> Vec<Hash> {
	touched.sort_unstable_by_key(|(para_id, _, _)| para_id.0);
	touched
		.iter()
		.filter_map(|(para_id, prior, current)| {
			let (current_head, leaf_head) = (*current)?;
			(Some(current_head) != *prior)
				.then(|| MerkleTree::Leaf { para_id: *para_id, head_hash: leaf_head }.hash())
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn leaf(para_id: u32, head_hash: Hash) -> Hash {
		MerkleTree::Leaf { para_id: ParaId(para_id), head_hash }.hash()
	}

	fn node(left: Hash, right: Hash) -> Hash {
		MerkleTree::Node(left, right).hash()
	}

	#[test]
	fn element_encoded_sizes_work() {
		// §5.5 pins both widths; they are what makes the 4-octet `para_id` and the
		// covered discriminant verifiable by an external checker.
		assert_eq!(MerkleTree::Leaf { para_id: ParaId(7), head_hash: [1; 32] }.encode().len(), 37);
		assert_eq!(MerkleTree::Node([1; 32], [2; 32]).encode().len(), 65);
	}

	#[test]
	fn leaf_node_images_are_disjoint_works() {
		// Covering the discriminant is what stops a leaf hash ever equalling a node
		// hash, so a proof cannot reinterpret one as the other (§5.5).
		let a = leaf(1, [1; 32]);
		let b = leaf(2, [2; 32]);
		for l in [a, b] {
			for r in [a, b] {
				assert_ne!(node(l, r), a);
				assert_ne!(node(l, r), b);
			}
		}
	}

	#[test]
	fn root_shape_works() {
		let l = leaf(1, [9; 32]);
		assert_eq!(merkle_root(alloc::vec![]), None);
		// A single leaf is its own root, not hashed again.
		assert_eq!(merkle_root(alloc::vec![l]), Some(l));
		assert_eq!(merkle_root(alloc::vec![l, l]), Some(node(l, l)));
	}

	#[test]
	fn odd_level_promotes_works() {
		// D-12: a trailing odd element is promoted unchanged, not duplicated.
		let (a, b, c) = ([1; 32], [2; 32], [3; 32]);
		assert_eq!(pair_up(&[a, b, c]), alloc::vec![node(a, b), c]);
		assert_eq!(merkle_root(alloc::vec![a, b, c]), Some(node(node(a, b), c)));
	}

	#[test]
	fn four_leaves_balance_works() {
		let (a, b, c, d) = ([1; 32], [2; 32], [3; 32], [4; 32]);
		assert_eq!(merkle_root(alloc::vec![a, b, c, d]), Some(node(node(a, b), node(c, d))));
	}

	#[test]
	fn unchanged_head_contributes_no_leaf_works() {
		let h: Hash = [5; 32];
		assert!(changed_leaves(alloc::vec![(ParaId(1), Some(h), Some((h, [6; 32])))]).is_empty());
	}

	#[test]
	fn newly_registered_para_contributes_leaf_works() {
		let h: Hash = [5; 32];
		assert_eq!(
			changed_leaves(alloc::vec![(ParaId(1), None, Some(([4; 32], h)))]),
			alloc::vec![leaf(1, h)]
		);
	}

	#[test]
	fn removed_para_contributes_no_leaf_works() {
		// `parachain_clean_up` leaves the para with no ending head value.
		assert!(changed_leaves(alloc::vec![(ParaId(1), Some([5; 32]), None)]).is_empty());
	}

	#[test]
	fn leaves_order_by_para_id_works() {
		// Ordering must not depend on the order the heads were touched (§5.5).
		let (h1, h4, h7): (Hash, Hash, Hash) = ([1; 32], [4; 32], [7; 32]);
		assert_eq!(
			changed_leaves(alloc::vec![
				(ParaId(7), None, Some(([3; 32], h7))),
				(ParaId(1), None, Some(([2; 32], h1))),
				(ParaId(4), None, Some(([3; 32], h4))),
			]),
			alloc::vec![leaf(1, h1), leaf(4, h4), leaf(7, h7)]
		);
	}

	#[test]
	fn single_changed_head_root_is_its_leaf_works() {
		let h: Hash = [9; 32];
		let leaves = changed_leaves(alloc::vec![
			(ParaId(2), Some([1; 32]), Some(([8; 32], h))),
			(ParaId(3), Some([2; 32]), Some(([2; 32], [8; 32]))),
		]);
		assert_eq!(merkle_root(leaves), Some(leaf(2, h)));
	}
}
