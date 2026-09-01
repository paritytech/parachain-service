//! Verifier tests driven by an independently-written reference trie builder.
//!
//! The builder below constructs nodes and roots straight from the Gray Paper's merklization rules
//! rather than reusing anything from the verifier, so a mistake shared by both is unlikely to go
//! unnoticed. `root_matches_polkajam` then pins the builder itself against polkajam's trie, which
//! makes the whole chain — builder, roots, and the proofs the verifier consumes — trustworthy.

use jam_state_helpers::{blake2_256, verify, Hash, ProofError, ProofNode, StateKey, StateProof};

const EMPTY_HASH: Hash = [0u8; 32];

/// A trie built from a full key/value set, able to emit a proof for any key.
struct Trie {
	entries: Vec<(StateKey, Vec<u8>)>,
	/// Every node created while hashing the trie, which is the pool a proof draws from.
	nodes: Vec<ProofNode>,
	root: Hash,
}

impl Trie {
	fn new(mut entries: Vec<(StateKey, Vec<u8>)>) -> Self {
		entries.sort_by(|(a, _), (b, _)| a.cmp(b));
		let mut trie = Trie { entries: entries.clone(), nodes: Vec::new(), root: EMPTY_HASH };
		trie.root = trie.hash_subtree(0, &entries);
		trie
	}

	/// The identity of the subtree at `depth` holding exactly `entries`.
	fn hash_subtree(&mut self, depth: usize, entries: &[(StateKey, Vec<u8>)]) -> Hash {
		match entries {
			// An empty subtree is the zero hash, and has no node.
			[] => EMPTY_HASH,
			// A lone entry collapses to a leaf, however deep the branch would have gone.
			[(key, value)] => self.push(leaf_node(key, value)),
			_ => {
				let (left, right): (Vec<_>, Vec<_>) =
					entries.iter().cloned().partition(|(key, _)| bit_at(key, depth) == 0);
				let left = self.hash_subtree(depth + 1, &left);
				let right = self.hash_subtree(depth + 1, &right);
				self.push(branch_node(&left, &right))
			},
		}
	}

	fn push(&mut self, node: ProofNode) -> Hash {
		let hash = blake2_256(&node);
		self.nodes.push(node);
		hash
	}

	/// A proof usable for any key: every node, plus preimages for large values.
	///
	/// Handing over the whole node pool is deliberate — the verifier is specified to tolerate
	/// extra nodes, since the trusted root is what constrains which of them it will look at.
	fn proof(&self) -> StateProof {
		StateProof {
			nodes: self.nodes.clone(),
			values: self
				.entries
				.iter()
				.filter(|(_, value)| value.len() > 32)
				.map(|(k, v)| (*k, v.clone()))
				.collect(),
		}
	}
}

fn bit_at(key: &StateKey, depth: usize) -> u8 {
	(key[depth / 8] >> (7 - (depth % 8))) & 1
}

fn leaf_node(key: &StateKey, value: &[u8]) -> ProofNode {
	let mut node = [0u8; 64];
	node[1..32].copy_from_slice(key);
	if value.len() > 32 {
		node[0] = 0b1100_0000;
		node[32..].copy_from_slice(&blake2_256(value));
	} else {
		node[0] = 0b1000_0000 | value.len() as u8;
		node[32..32 + value.len()].copy_from_slice(value);
	}
	node
}

fn branch_node(left: &Hash, right: &Hash) -> ProofNode {
	let mut node = [0u8; 64];
	node[..32].copy_from_slice(left);
	node[32..].copy_from_slice(right);
	// The branch discriminator claims the top bit of the left child hash.
	node[0] &= 0b0111_1111;
	node
}

fn key(first: u8) -> StateKey {
	let mut key = [0u8; 31];
	key[0] = first;
	key
}

/// A parachain-service para-head key, the shape this crate actually serves in production.
fn para_head_key(service_id: u32, para_id: u32) -> StateKey {
	let mut service_local = vec![0x00];
	service_local.extend_from_slice(&para_id.to_le_bytes());
	jam_state_helpers::service_value_state_key(service_id, &service_local)
}

#[test]
fn single_entry_works() {
	let entries = vec![(key(0x00), b"the para head".to_vec())];
	let trie = Trie::new(entries);

	let found = verify(&trie.proof(), &trie.root, &key(0x00)).expect("valid proof");
	assert_eq!(found.as_deref(), Some(b"the para head".as_slice()));
}

#[test]
fn many_entries_works() {
	// Keys chosen to branch at the very first bit and at deeper ones, so the walk has to descend
	// through several branch levels rather than hitting a collapsed leaf immediately.
	let entries: Vec<_> =
		[0x00u8, 0x01, 0x02, 0x40, 0x80, 0xc0, 0xff].iter().map(|b| (key(*b), vec![*b; 4])).collect();
	let trie = Trie::new(entries.clone());

	for (k, expected) in &entries {
		let found = verify(&trie.proof(), &trie.root, k).expect("valid proof");
		assert_eq!(found.as_ref(), Some(expected), "key {:02x}", k[0]);
	}
}

/// The genesis case: the first parachain block is accepted precisely because the proof shows the
/// para has no head yet, so absence must verify as a success and not an error.
#[test]
fn absence_in_empty_trie_works() {
	let trie = Trie::new(Vec::new());
	assert_eq!(trie.root, EMPTY_HASH);
	assert_eq!(verify(&trie.proof(), &trie.root, &key(0x00)), Ok(None));
}

/// Absence has to be provable in a populated trie too — one para having a head says nothing about
/// another, and the second para's first block still needs to be accepted.
#[test]
fn absence_beside_other_entries_works() {
	let service = 9;
	let stored = para_head_key(service, 0);
	let missing = para_head_key(service, 1);
	let trie = Trie::new(vec![(stored, b"head of para 0".to_vec())]);

	assert_eq!(verify(&trie.proof(), &trie.root, &missing), Ok(None));
	assert!(verify(&trie.proof(), &trie.root, &stored).expect("valid").is_some());
}

#[test]
fn absence_with_many_entries_works() {
	let entries: Vec<_> = [0x00u8, 0x01, 0x80, 0xff].iter().map(|b| (key(*b), vec![*b])).collect();
	let trie = Trie::new(entries);

	for absent in [0x02u8, 0x40, 0x7f, 0xc0] {
		assert_eq!(
			verify(&trie.proof(), &trie.root, &key(absent)),
			Ok(None),
			"key {absent:02x}"
		);
	}
}

/// A value too long to sit inside its leaf is committed to by hash, so the preimage travels
/// separately and must be checked against that commitment.
#[test]
fn large_value_works() {
	let value = vec![7u8; 100];
	let trie = Trie::new(vec![(key(0x00), value.clone())]);

	let found = verify(&trie.proof(), &trie.root, &key(0x00)).expect("valid proof");
	assert_eq!(found, Some(value));
}

/// Exactly 32 octets is the largest value a leaf still holds inline; 33 is the first that does
/// not. Both sides of that boundary must round-trip.
#[test]
fn value_length_boundary_works() {
	for len in [0usize, 1, 31, 32, 33, 64] {
		let value = vec![0xabu8; len];
		let trie = Trie::new(vec![(key(0x00), value.clone())]);
		let found = verify(&trie.proof(), &trie.root, &key(0x00)).expect("valid proof");
		assert_eq!(found, Some(value), "length {len}");
	}
}

/// The point of the whole exercise: a proof that does not belong to the trusted root must not
/// verify. Otherwise a dropped work package could be replaced by a forged ancestry.
#[test]
fn wrong_root_errors() {
	let trie = Trie::new(vec![(key(0x00), b"real head".to_vec())]);
	let forged = Trie::new(vec![(key(0x00), b"forged head".to_vec())]);

	assert_eq!(
		verify(&forged.proof(), &trie.root, &key(0x00)),
		Err(ProofError::IncompleteProof),
	);
}

/// Tampering with a node changes its hash, so it no longer answers to the identity its parent
/// committed to and the walk can no longer proceed.
#[test]
fn tampered_node_errors() {
	let entries: Vec<_> = [0x00u8, 0x80].iter().map(|b| (key(*b), vec![*b])).collect();
	let trie = Trie::new(entries);

	let mut proof = trie.proof();
	proof.nodes[0][40] ^= 0xff;

	assert_eq!(verify(&proof, &trie.root, &key(0x00)), Err(ProofError::IncompleteProof));
}

/// An empty proof must never be taken as evidence of absence: that would let anyone claim any key
/// is unset, which for parasim would mean forging the genesis case at will.
#[test]
fn empty_proof_errors() {
	let trie = Trie::new(vec![(key(0x00), b"head".to_vec())]);
	let empty = StateProof { nodes: Vec::new(), values: Vec::new() };

	assert_eq!(verify(&empty, &trie.root, &key(0x00)), Err(ProofError::IncompleteProof));
}

/// Withholding the leaf while supplying the branches must not read as absence either.
#[test]
fn truncated_proof_errors() {
	let entries: Vec<_> = [0x00u8, 0x80].iter().map(|b| (key(*b), vec![*b])).collect();
	let trie = Trie::new(entries);

	let mut proof = trie.proof();
	let target = leaf_node(&key(0x00), &[0x00]);
	proof.nodes.retain(|node| node != &target);

	assert_eq!(verify(&proof, &trie.root, &key(0x00)), Err(ProofError::IncompleteProof));
}

/// A large leaf whose preimage is missing, or does not match, must not yield a value.
#[test]
fn bad_large_value_errors() {
	let trie = Trie::new(vec![(key(0x00), vec![7u8; 100])]);

	let mut missing = trie.proof();
	missing.values.clear();
	assert_eq!(verify(&missing, &trie.root, &key(0x00)), Err(ProofError::ValueMismatch));

	let mut wrong = trie.proof();
	wrong.values[0].1 = vec![8u8; 100];
	assert_eq!(verify(&wrong, &trie.root, &key(0x00)), Err(ProofError::ValueMismatch));
}

/// Extra nodes are explicitly tolerated: the trusted root decides which nodes are consulted, so
/// padding a proof cannot change its verdict. This keeps us off a minimality rule that the
/// collator side would have to match exactly.
#[test]
fn extra_nodes_are_ignored() {
	let trie = Trie::new(vec![(key(0x00), b"head".to_vec())]);
	let unrelated = Trie::new(vec![(key(0x77), b"noise".to_vec())]);

	let mut proof = trie.proof();
	proof.nodes.extend(unrelated.nodes.clone());

	let found = verify(&proof, &trie.root, &key(0x00)).expect("valid proof");
	assert_eq!(found.as_deref(), Some(b"head".as_slice()));
}

/// Pins the reference builder — and therefore every root the tests above assert against — to
/// polkajam's trie. Without this the suite could be self-consistently wrong.
#[test]
fn root_matches_polkajam() {
	let cases: Vec<Vec<(StateKey, Vec<u8>)>> = vec![
		Vec::new(),
		vec![(key(0x00), b"head".to_vec())],
		vec![(key(0x00), Vec::new())],
		vec![(key(0x00), vec![9u8; 32])],
		vec![(key(0x00), vec![9u8; 33])],
		vec![(key(0x00), vec![1]), (key(0x80), vec![2])],
		vec![(key(0x00), vec![1]), (key(0x01), vec![2])],
		[0x00u8, 0x01, 0x02, 0x40, 0x80, 0xc0, 0xff]
			.iter()
			.map(|b| (key(*b), vec![*b; 40]))
			.collect(),
		vec![
			(para_head_key(9, 0), b"head of para 0".to_vec()),
			(para_head_key(9, 1), b"head of para 1".to_vec()),
			(para_head_key(10, 0), b"another service".to_vec()),
		],
	];

	for (index, entries) in cases.iter().enumerate() {
		let trie = polkajam_trie::Trie::new_empty();
		let commit = trie
			.commit(
				polkajam_trie::RootId::empty(),
				entries
					.iter()
					.map(|(k, v)| (*k, Some(polkajam_trie::Value::new(v)))),
			)
			.expect("committing to a fresh trie succeeds");

		assert_eq!(Trie::new(entries.clone()).root, commit.root_hash, "case {index}");
	}
}
