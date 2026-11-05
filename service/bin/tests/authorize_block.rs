//! The authorizer's `authorize_block_ext` entry point, run on the real ed25519 authorizer blob
//! through `parachain_service_core::authorize_block`, the way a parachain runtime runs it.

use codec::Encode;
use ed25519_dalek::{Signer, SigningKey};
use jam_types::{BoundedVec, Encode as _};
use parachain_authorizer::{aura, block::signable_block_hash};
use parachain_service_bin::authorizer_blob;
use parachain_service_core::{
	authorize_block::{authorize_block, AuthorizeBlockError, RunError},
	blake2_256,
	block_authorization::{BlockAuthorization, AUTH_POOLS_STATE_KEY},
	service_preimage_state_key,
	types::ParaId,
	StateKey,
};
use primitive_types::H256;
use std::collections::HashMap;

const PARA: u32 = 100;
const CODE_HOST: u32 = 7;
const BLOCK_HASH: [u8; 32] = [42; 32];
/// Names collator 0 of two, at one slot per turn.
const SLOT: u32 = 4;

struct Collators {
	keys: Vec<SigningKey>,
	proofs: Vec<Vec<H256>>,
	root: H256,
}

fn collators() -> Collators {
	let keys: Vec<_> = (1..=2u8).map(|i| SigningKey::from_bytes(&[i; 32])).collect();
	let public: Vec<[u8; 32]> = keys.iter().map(|k| k.verifying_key().to_bytes()).collect();
	let (proofs, roots): (Vec<_>, Vec<_>) = (0..public.len() as u32)
		.map(|i| {
			let (root, proof) = aura::build_collator_proof(&public, i).expect("valid collator set");
			(proof, root)
		})
		.unzip();
	Collators { keys, proofs, root: roots[0] }
}

/// A block authorization by collator `signer` over `signed`.
fn authorization(blob: &[u8], signer: usize, signed: H256) -> BlockAuthorization {
	let collators = collators();
	let config = aura::AuthConfig {
		para_ids: vec![PARA.into()],
		parachain_service: 1337,
		collator_set_root: collators.root,
		collator_set_size: 2.try_into().unwrap(),
		slot_duration: 1.try_into().unwrap(),
	};
	let key = &collators.keys[signer];
	let token = aura::AuthToken {
		proof: collators.proofs[signer].clone(),
		key: key.verifying_key().to_bytes(),
		signature: key.sign(signed.as_bytes()).to_bytes(),
	};
	BlockAuthorization {
		auth_code_host: CODE_HOST,
		code_hash: blake2_256(blob),
		config: config.encode(),
		token: token.encode(),
	}
}

/// JAM state at the anchor: `pooled` in core 1's auth pool, the authorizer code as a preimage.
fn state(blob: &[u8], pooled: &BlockAuthorization) -> HashMap<StateKey, Vec<u8>> {
	let mut pools = jam_std_common::AuthPools::default();
	pools[1] = BoundedVec::truncate_from(vec![pooled.authorizer_hash()]);

	HashMap::from([
		(AUTH_POOLS_STATE_KEY, pools.encode()),
		(service_preimage_state_key(CODE_HOST, &blake2_256(blob)), blob.to_vec()),
	])
}

fn check(
	state: &HashMap<StateKey, Vec<u8>>,
	para: u32,
	auth: &BlockAuthorization,
) -> Result<Vec<u8>, AuthorizeBlockError> {
	authorize_block(|key| state.get(key).cloned(), ParaId::from(para), auth, BLOCK_HASH, SLOT)
}

#[test]
fn trivial_works() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 0, signable_block_hash(&BLOCK_HASH));

	let trace = check(&state(&blob, &auth), PARA, &auth).expect("block is authorized");

	let author_key = collators().keys[0].verifying_key().to_bytes();
	assert_eq!(trace, aura::AuthTrace { author_key }.encode());
}

#[test]
fn not_in_any_pool_errors() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 0, signable_block_hash(&BLOCK_HASH));
	let other = BlockAuthorization { config: vec![0], ..auth.clone() };

	assert_eq!(check(&state(&blob, &other), PARA, &auth), Err(AuthorizeBlockError::NotInAnyPool));
}

#[test]
fn foreign_para_errors() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 0, signable_block_hash(&BLOCK_HASH));

	assert_eq!(check(&state(&blob, &auth), PARA + 1, &auth), Err(AuthorizeBlockError::ForeignPara));
}

#[test]
fn code_unavailable_errors() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 0, signable_block_hash(&BLOCK_HASH));
	let mut state = state(&blob, &auth);
	state.remove(&service_preimage_state_key(CODE_HOST, &auth.code_hash));

	assert_eq!(check(&state, PARA, &auth), Err(AuthorizeBlockError::CodeUnavailable));
}

#[test]
fn code_hash_mismatch_errors() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 0, signable_block_hash(&BLOCK_HASH));
	let mut state = state(&blob, &auth);
	state.insert(service_preimage_state_key(CODE_HOST, &auth.code_hash), blob[1..].to_vec());

	assert_eq!(check(&state, PARA, &auth), Err(AuthorizeBlockError::CodeHashMismatch));
}

/// A signature in the work-package domain never authorizes a block.
#[test]
fn work_package_signature_errors() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 0, aura::hash_signing_payload(&BLOCK_HASH));

	assert_eq!(
		check(&state(&blob, &auth), PARA, &auth),
		Err(AuthorizeBlockError::Run(RunError::Trap))
	);
}

#[test]
fn other_block_signature_errors() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 0, signable_block_hash(&[43; 32]));

	assert_eq!(
		check(&state(&blob, &auth), PARA, &auth),
		Err(AuthorizeBlockError::Run(RunError::Trap))
	);
}

/// Collator 1 is in the set, but the slot names collator 0.
#[test]
fn collator_not_named_by_slot_errors() {
	let blob = authorizer_blob();
	let auth = authorization(&blob, 1, signable_block_hash(&BLOCK_HASH));

	assert_eq!(
		check(&state(&blob, &auth), PARA, &auth),
		Err(AuthorizeBlockError::Run(RunError::Trap))
	);
}

#[test]
fn malformed_token_errors() {
	let blob = authorizer_blob();
	let auth = BlockAuthorization {
		token: vec![1, 2, 3],
		..authorization(&blob, 0, signable_block_hash(&BLOCK_HASH))
	};

	assert_eq!(
		check(&state(&blob, &auth), PARA, &auth),
		Err(AuthorizeBlockError::Run(RunError::Trap))
	);
}
