//! Collator membership proofs, including padded trees and invalid membership.

use jp_aura_authorizer::{build_collator_tree, AuthConfig, AuthToken, CollatorKey, ParaId};
use primitive_types::H256;

fn keys(count: u32) -> Vec<CollatorKey> {
	(0..count).map(|index| [index as u8 + 1; 32]).collect()
}

fn config(keys: &[CollatorKey]) -> (AuthConfig, Vec<Vec<H256>>) {
	let (collator_set_root, proofs) = build_collator_tree(keys);
	let config = AuthConfig {
		para_ids: vec![ParaId::new(0)],
		parachain_service: 1337,
		collator_set_root,
		collator_set_size: keys.len() as u32,
		slot_duration: 1,
	};
	(config, proofs)
}

fn token(key: CollatorKey, proof: Vec<H256>) -> AuthToken {
	AuthToken { proof, key, signature: [0u8; 64] }
}

#[test]
fn built_proofs_works() {
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

#[test]
fn wrong_index_errors() {
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
fn nonmember_errors() {
	let keys = keys(2);
	let (config, proofs) = config(&keys);
	let outsider = token([0xaa; 32], proofs[0].clone());
	assert!(outsider.check_proof(&config, 0).is_err());
}
