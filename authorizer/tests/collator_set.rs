//! The two halves of the collator set that only agree by construction: the trie the authorizer
//! verifies proofs against, and the builder that produces those proofs.
//!
//! All of this is scheme-blind — the trie hashes raw key bytes. The signature half of the token
//! is in `contract.rs`, where both schemes are exercised.

use parachain_authorizer::aura::{build_collator_tree, AuthConfig, AuthToken, CollatorKey};
use parachain_service_interface::types::ParaId;
use primitive_types::H256;

fn keys(count: u32) -> Vec<CollatorKey> {
	(0..count).map(|index| [index as u8 + 1; 32]).collect()
}

fn config(keys: &[CollatorKey]) -> (AuthConfig, Vec<Vec<H256>>) {
	let (collator_set_root, proofs) = build_collator_tree(keys);
	let config = AuthConfig {
		para_ids: vec![ParaId(0)],
		parachain_service: 5,
		collator_set_root,
		collator_set_size: keys.len() as u32,
		slot_duration: 1,
	};
	(config, proofs)
}

fn token(key: CollatorKey, proof: Vec<H256>) -> AuthToken {
	AuthToken { proof, key, signature: [0u8; 64], control_command: None }
}

/// The builder is the verifier's inverse, so every proof it hands out must satisfy the
/// verifier — at every set size, including the ones that pad the trie. A set of 3 pads to 4,
/// which is where a leaf-ordering or padding mistake shows up.
#[test]
fn every_proof_the_builder_makes_verifies_works() {
	for size in 1..=5u32 {
		let keys = keys(size);
		let (config, proofs) = config(&keys);
		for index in 0..size {
			let token = token(keys[index as usize], proofs[index as usize].clone());
			assert!(
				token.check_proof(&config, index).is_ok(),
				"set of {size}: collator {index}'s own proof was rejected"
			);
		}
	}
}

/// A proof only proves membership *at the index the round-robin named*. Accepting it at another
/// index would let any collator in the set author in everybody else's slot, which is the whole
/// point of the AURA schedule.
#[test]
fn a_proof_is_rejected_at_another_index_works() {
	let keys = keys(4);
	let (config, proofs) = config(&keys);
	let token = token(keys[1], proofs[1].clone());
	assert!(token.check_proof(&config, 1).is_ok());
	for index in [0u32, 2, 3] {
		assert!(token.check_proof(&config, index).is_err(), "accepted at index {index}");
	}
}

/// A key outside the set has no proof, and neither the root nor a borrowed proof can supply one.
#[test]
fn an_outsider_cannot_prove_membership_works() {
	let keys = keys(2);
	let (config, proofs) = config(&keys);
	let outsider = token([0xaa; 32], proofs[0].clone());
	assert!(outsider.check_proof(&config, 0).is_err());
}
