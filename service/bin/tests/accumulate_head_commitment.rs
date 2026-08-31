//! Head-commitment integration tests (§5.5), through the real PVM blob.
//!
//! `accumulate` returns the Merkle root over the parachain heads that changed
//! during the block (service/src/accumulate/mod.rs), ordered by ascending
//! `para_id`, or `None` when none did. These tests drive real work items through
//! the compiled service blob and check the surfaced `yielded` commitment against
//! an independently recomputed tree, mirroring the pure-Rust unit tests in
//! service/src/head_commitment.rs.

mod common;

use codec::Encode;
use common::*;
use parachain_service::hashing::keccak_256;
use parachain_service_interface::types::{Hash, ParaId};
use tiny_keccak::{Hasher as _, Keccak};

const NOW: u32 = 100;
const PARA_A: ParaId = ParaId(1);
const PARA_B: ParaId = ParaId(4);
const PARA_C: ParaId = ParaId(7);
const CODE_A: &[u8] = b"code-a";
const CODE_B: &[u8] = b"code-b";
const CODE_C: &[u8] = b"code-c";

/// §5.5 tree element: keccak-256 over the SCALE encoding, so the variant
/// discriminant is covered and a leaf hash can never collide with a node hash.
/// Mirrors `service/src/head_commitment.rs::MerkleTree` exactly.
#[derive(Encode)]
enum MerkleTree {
	Node(Hash, Hash),
	Leaf { para_id: ParaId, head_hash: Hash },
}

fn element_hash(element: &MerkleTree) -> Hash {
	let mut keccak = Keccak::v256();
	keccak.update(&element.encode());
	let mut out = Hash::default();
	keccak.finalize(&mut out);
	out
}

fn leaf(para_id: ParaId, head: &[u8]) -> Hash {
	element_hash(&MerkleTree::Leaf { para_id, head_hash: keccak_256(head) })
}

fn node(left: Hash, right: Hash) -> Hash {
	element_hash(&MerkleTree::Node(left, right))
}

/// Root over `leaves`, collapsing adjacent pairs and promoting a trailing odd
/// element unchanged (D-12) — mirrors `head_commitment.rs::pair_up`/`merkle_root`.
fn root(mut leaves: Vec<Hash>) -> Option<Hash> {
	while leaves.len() > 1 {
		let mut chunks = leaves.chunks_exact(2);
		let mut next: Vec<Hash> = chunks.by_ref().map(|pair| node(pair[0], pair[1])).collect();
		if let [odd] = chunks.remainder() {
			next.push(*odd);
		}
		leaves = next;
	}
	leaves.first().copied()
}

#[test]
fn single_changed_head_commits_to_its_leaf_works() {
	let storage = fresh_storage(|s| seed_para(s, PARA_A, b"genesis", CODE_A, RICH));
	let digest = ok_digest(PARA_A, CODE_A, b"genesis", b"head-1", vec![], 0);

	let (outcome, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(&para_info(&storage, PARA_A).unwrap().head_data[..], b"head-1");
	// §5.5: one changed head → the returned commitment is that leaf's own hash.
	assert_eq!(outcome.yielded, Some(leaf(PARA_A, b"head-1")));
}

#[test]
fn no_changed_head_returns_no_commitment_works() {
	let storage = fresh_storage(|s| seed_para(s, PARA_A, b"genesis", CODE_A, RICH));
	let digest = ok_digest(PARA_A, CODE_A, b"genesis", b"head-1", vec![], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	// Same head again: parent check passes, but the head is unchanged → no leaf.
	let digest = ok_digest(PARA_A, CODE_A, b"head-1", b"head-1", vec![], 0);
	let (outcome, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);
	assert_eq!(outcome.yielded, None);

	// An operand-less block (only always-accumulate work) moves no head either.
	let (outcome, _, _) = accumulate_block(storage, vec![], NOW + 2);
	assert_eq!(outcome.yielded, None);
}

#[test]
fn multiple_changed_heads_order_by_para_id_works() {
	let storage = fresh_storage(|s| {
		seed_para(s, PARA_A, b"genesis-a", CODE_A, RICH);
		seed_para(s, PARA_B, b"genesis-b", CODE_B, RICH);
	});
	// Distinct head values per para: a swapped leaf order would change the root.
	let digest_a = ok_digest(PARA_A, CODE_A, b"genesis-a", b"head-a1", vec![], 0);
	let digest_b = ok_digest(PARA_B, CODE_B, b"genesis-b", b"head-b1", vec![], 0);

	// Touched in descending para_id order; the root must still order leaves ascending.
	let (outcome, storage, _) =
		accumulate_block(storage, vec![work_item(&digest_b), work_item(&digest_a)], NOW);

	assert_eq!(&para_info(&storage, PARA_A).unwrap().head_data[..], b"head-a1");
	assert_eq!(&para_info(&storage, PARA_B).unwrap().head_data[..], b"head-b1");
	let expected = root(vec![leaf(PARA_A, b"head-a1"), leaf(PARA_B, b"head-b1")]);
	assert_eq!(outcome.yielded, expected);
}

#[test]
fn three_changed_heads_promote_odd_leaf_works() {
	// D-12: with three leaves, the first pair is hashed and the trailing odd
	// element is promoted unchanged — not duplicated.
	let storage = fresh_storage(|s| {
		seed_para(s, PARA_A, b"genesis-a", CODE_A, RICH);
		seed_para(s, PARA_B, b"genesis-b", CODE_B, RICH);
		seed_para(s, PARA_C, b"genesis-c", CODE_C, RICH);
	});
	let digest_a = ok_digest(PARA_A, CODE_A, b"genesis-a", b"head-a1", vec![], 0);
	let digest_b = ok_digest(PARA_B, CODE_B, b"genesis-b", b"head-b1", vec![], 0);
	let digest_c = ok_digest(PARA_C, CODE_C, b"genesis-c", b"head-c1", vec![], 0);

	let (outcome, storage, _) = accumulate_block(
		storage,
		vec![work_item(&digest_c), work_item(&digest_a), work_item(&digest_b)],
		NOW,
	);

	assert_eq!(&para_info(&storage, PARA_A).unwrap().head_data[..], b"head-a1");
	assert_eq!(&para_info(&storage, PARA_B).unwrap().head_data[..], b"head-b1");
	assert_eq!(&para_info(&storage, PARA_C).unwrap().head_data[..], b"head-c1");
	let expected =
		root(vec![leaf(PARA_A, b"head-a1"), leaf(PARA_B, b"head-b1"), leaf(PARA_C, b"head-c1")]);
	assert_eq!(outcome.yielded, expected);
}
