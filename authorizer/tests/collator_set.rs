//! The two halves of the token that only agree by construction: the collator-set trie the
//! authorizer verifies against and the builder that produces it, and the payload a collator
//! signs and the one `check_signature` recomputes.

use ed25519_dalek::{Signer as _, SigningKey};
use parachain_authorizer::aura::{
	build_collator_tree, AuthConfig, AuthToken, CollatorKey, Command,
};
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

/// The control command travels in the token, and the package hash deliberately excludes the
/// token — so unless the signature covers the command, anyone can bolt one onto a package they
/// intercept in flight and reassign a core with somebody else's signature.
#[test]
fn a_signature_does_not_carry_over_to_another_command_works() {
	let signing_key = SigningKey::from_bytes(&[7u8; 32]);
	let key = signing_key.verifying_key().to_bytes();
	let wp_hash = H256::repeat_byte(0xab);
	let command = Command::Assign { para_id: ParaId(3), core: 1, authorizer: [0xcd; 32] };

	let sign = |command: &Option<Command>| {
		signing_key
			.sign(AuthToken::signing_payload(wp_hash, command).as_bytes())
			.to_bytes()
	};
	let plain = AuthToken { signature: sign(&None), ..token(key, vec![]) };
	let commanding = AuthToken { signature: sign(&Some(command.clone())), ..token(key, vec![]) };

	assert!(plain.check_signature(wp_hash).is_ok());
	assert!(commanding.check_signature(wp_hash).is_err(), "the signature is for no command");
	assert!(AuthToken { control_command: Some(command.clone()), ..plain }
		.check_signature(wp_hash)
		.is_err());
	assert!(AuthToken { control_command: Some(command), ..commanding }
		.check_signature(wp_hash)
		.is_ok());
}
