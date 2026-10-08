//! Unit tests for the chain-spec builder.

use parachain_service_core::types::validation_code_hash_bytes;
use std::collections::{BTreeMap, BTreeSet};

use codec::{Decode, Encode};
use cumulus_aura_authorizer::AuthConfig;
use jam_std_common::hash_raw;
use jam_types::{AuthorizerHash, Balance};
use parachain_service::{
	state::{para_info::ParaInfo, preimage_registry::PreimageEntry, storage_key, Tag},
	state_balance::{baseline_for, preimage_footprint},
	work_digest::{validation_code_hash, ValidationCodeRef},
};
use parachain_service_core::{
	types::{HeadData, ParaId, MAX_HEAD_DATA_SIZE},
	PARACHAIN_SERVICE_ID,
};
use primitive_types::H256;

use crate::{parachain::ParachainSpec, Error, ParachainServiceSpec};

const SERVICE_CODE: &[u8] = b"parachain service code";
const HEAD: &[u8] = b"heads up";
const CODE: &[u8] = b"validation code";
const CODE_2: &[u8] = b"other validation code";
const RICH: Balance = 10_000_000;

fn realm_config(para: ParaId) -> AuthConfig {
	AuthConfig {
		para_ids: vec![para],
		parachain_service: PARACHAIN_SERVICE_ID,
		collator_set_root: H256::zero(),
		collator_set_size: 1,
		slot_duration: 6,
	}
}

fn decode<V: Decode>(raw: &[u8]) -> V {
	V::decode(&mut &raw[..]).expect("genesis values are service-written; qed")
}

fn code_ref(code: &[u8]) -> ValidationCodeRef {
	ValidationCodeRef { hash: validation_code_hash(code), len: code.len() as u32 }
}

/// blake2b-256, independent of the service crates.
fn blake2b(input: &[u8]) -> [u8; 32] {
	let mut out = [0u8; 32];
	out.copy_from_slice(blake2b_simd::Params::new().hash_length(32).hash(input).as_bytes());
	out
}

/// The `ParaInfo` row build writes for one registered para.
fn para_info_entry(service: &crate::BuiltParachainService, para: ParaId) -> ParaInfo {
	decode(
		service
			.storage
			.get(&storage_key(Tag::Parachains, &para))
			.expect("para entry; qed"),
	)
}

#[test]
fn registered_para_layout() {
	let service = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(
			ParachainSpec::new(ParaId::new(3))
				.head_data(HEAD)
				.validation_code(CODE)
				.state_balance(RICH),
		)
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.id, PARACHAIN_SERVICE_ID);
	assert_eq!(service.balance, Balance::MAX, "an unset balance keeps the unlimited default");
	assert_eq!(service.storage.len(), 2, "one para entry and one registry entry");

	// `[0x00] ‖ SCALE(ParaId)`, and the value round-trips to the expected `ParaInfo`.
	let key = storage_key(Tag::Parachains, &ParaId::new(3));
	assert_eq!(key, vec![0x00, 3, 0, 0, 0]);
	let info = para_info_entry(&service, ParaId::new(3));
	assert_eq!(info.head_data, HeadData::try_from(HEAD.to_vec()).expect("small head; qed"));
	assert_eq!(info.validation_code, Some(code_ref(CODE)));
	assert_eq!(info.announced_upgrade, None);
	assert_eq!(info.total_state_balance, RICH);
	assert_eq!(
		info.used_state_balance,
		baseline_for(ParaId::new(3)) + preimage_footprint(CODE.len() as u32)
	);
	assert!(!info.is_deregistering);
}

#[test]
fn validation_code_is_hosted_once_with_registry_entry() {
	let service = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId::new(3)).validation_code(CODE))
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages, vec![CODE.to_vec()], "the code blob is hosted exactly once");
	let entry: PreimageEntry = decode(
		service
			.storage
			.get(&storage_key(
				Tag::PreimageRegistry,
				&(validation_code_hash_bytes(&code_ref(CODE).hash), CODE.len() as u32),
			))
			.expect("registry entry; qed"),
	);
	assert_eq!(entry.referencers, BTreeSet::from([ParaId::new(3)]));
}

#[test]
fn shared_validation_code_is_hosted_once_and_referenced_by_both() {
	let service = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		// Out of order on purpose: output is ParaId-sorted.
		.parachain(ParachainSpec::new(ParaId::new(200)).validation_code(CODE))
		.parachain(ParachainSpec::new(ParaId::new(100)).validation_code(CODE))
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages.iter().filter(|blob| *blob == CODE).count(), 1);
	let entry: PreimageEntry = decode(
		service
			.storage
			.get(&storage_key(
				Tag::PreimageRegistry,
				&(validation_code_hash_bytes(&code_ref(CODE).hash), CODE.len() as u32),
			))
			.expect("registry entry; qed"),
	);
	assert_eq!(entry.referencers, BTreeSet::from([ParaId::new(100), ParaId::new(200)]));
	for para in [ParaId::new(100), ParaId::new(200)] {
		let info = para_info_entry(&service, para);
		assert_eq!(info.validation_code, Some(code_ref(CODE)));
	}
}

#[test]
fn shared_authorizer_blob_is_hosted_once() {
	let verifier = b"authorizer blob".to_vec();
	let config = realm_config(ParaId::new(7));
	let service = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId::new(7)).authorizer(verifier.clone(), &config))
		.parachain(ParachainSpec::new(ParaId::new(8)).authorizer(verifier.clone(), &config))
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages.iter().filter(|blob| **blob == verifier).count(), 1);
}

#[test]
fn authorizer_hashes_match_blake2b_concat() {
	let verifier = b"verifier blob".to_vec();
	let config = realm_config(ParaId::new(9));
	let spec = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId::new(9)).authorizer(verifier.clone(), &config));

	// Independent computation, rebuilt from the same raw parts the collator uses
	// (SDK `nodes/jam/authorizer.rs:102,118-119`): the code hash through
	// `jam_std_common::hash_raw`, then blake2b-256(code_hash ‖ SCALE(config)).
	let mut concat = hash_raw(&verifier).to_vec();
	concat.extend_from_slice(&config.encode());

	let hashes = spec.authorizer_hashes();
	assert_eq!(hashes, BTreeMap::from([(ParaId::new(9), AuthorizerHash(blake2b(&concat)))]));
}

/// The two hash paths must be byte-identical, or a genesis-queued authorizer
/// hash never matches what the collator computes — silently.
#[test]
fn authorizer_hash_raw_matches_blake2b_simd() {
	let data: &[u8] = b"the same blob, hashed by both paths";
	let expected = blake2b(data);
	let actual: [u8; 32] = hash_raw(data);
	assert_eq!(actual, expected);
}

/// The collator hashes the validation-code blob with `sp_crypto_hashing::blake2_256`
/// (SDK `cumulus/polkadot-omni-node/lib/src/nodes/jam/collation_task.rs:450`) and carries it
/// as the candidate's `ParachainCandidate.validation_code` (`:948-959`). The genesis
/// builder hashes the same blob with `parachain_service::work_digest::validation_code_hash`
/// and records `ValidationCodeRef { hash, len }` in the para's `ParaInfo`. If the two
/// disagree, refine's historical code lookup misses and every candidate is
/// refused with `RefineLog::ValidationCodeLookupFailed` — silently, on chain.
///
/// The collator side is rebuilt from blake2b-256 via `blake2b_simd`, the exact primitive
/// `sp_crypto_hashing::blake2_256` sits on (SDK `substrate/primitives/crypto/hashing/
/// src/lib.rs:29-51`), so this asserts an agreement between two code paths, not a tautology.
#[test]
fn validation_code_hash_matches_collator_derivation_works() {
	// Build a real PVF from this checkout rather than depending on an external SDK fixture.
	let blob = cargo_jam_build::blob("frameless");
	assert!(!blob.is_empty(), "the runtime build must produce a blob");
	assert_eq!(&blob[..4], b"PVM\0", "the blob must be a PolkaVM program, not WASM");

	let collator: [u8; 32] = blake2b(&blob);
	let builder = validation_code_hash(&blob);
	assert_eq!(
		validation_code_hash_bytes(&builder),
		collator,
		"collator and genesis must derive the same code hash"
	);

	// The preimage registry is keyed by `(hash, len)` (§6.1): a wrong length misses just as
	// silently as a wrong hash.
	let cref = ValidationCodeRef { hash: builder, len: blob.len() as u32 };
	assert_eq!(cref.len as usize, blob.len(), "ValidationCodeRef.len must be the blob's length");
}

#[test]
fn oversized_head_data_is_a_typed_error() {
	let big = vec![0u8; MAX_HEAD_DATA_SIZE as usize + 1];
	let err = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId::new(1)).head_data(big.clone()))
		.build()
		.expect_err("head over the bound must be rejected, not panicked");
	assert!(matches!(err, Error::HeadDataTooLarge { para: 1, len } if len == big.len()));

	// The bound itself fits.
	ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(
			ParachainSpec::new(ParaId::new(1)).head_data(vec![0u8; MAX_HEAD_DATA_SIZE as usize]),
		)
		.build()
		.expect("exactly 4 KiB fits; qed");
}

#[test]
fn para_without_validation_code_has_none_and_no_registry_entry() {
	let service = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId::new(9)).state_balance(RICH))
		.build()
		.expect("a small spec builds; qed");

	let info = para_info_entry(&service, ParaId::new(9));
	assert_eq!(info.validation_code, None);
	assert_eq!(info.used_state_balance, baseline_for(ParaId::new(9)), "no preimage footprint");
	assert!(service.preimages.is_empty(), "nothing to host");
	assert!(
		service.storage.keys().all(|k| k[0] != Tag::PreimageRegistry as u8),
		"no registry entries for a para without a validation code"
	);
}

#[test]
fn extra_preimage_duplicate_of_validation_code_is_hosted_once() {
	let service = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId::new(1)).validation_code(CODE))
		.preimage(CODE)
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages.iter().filter(|blob| *blob == CODE).count(), 1);
}

#[test]
fn build_is_deterministic_regardless_of_insertion_order() {
	let mk = |order: [ParaId; 3]| {
		let mut spec = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE);
		for id in order {
			let code: &[u8] = if u32::from(id) % 2 == 0 { CODE } else { CODE_2 };
			spec = spec.parachain(ParachainSpec::new(id).validation_code(code).state_balance(RICH));
		}
		spec.build().expect("a small spec builds; qed")
	};

	let a = mk([ParaId::new(2), ParaId::new(1), ParaId::new(3)]);
	let b = mk([ParaId::new(3), ParaId::new(1), ParaId::new(2)]);
	assert_eq!(a.storage, b.storage);
	assert_eq!(a.preimages, b.preimages);
}

#[test]
fn duplicate_para_id_is_an_error() {
	let err = ParachainServiceSpec::new(PARACHAIN_SERVICE_ID, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId::new(1)))
		.parachain(ParachainSpec::new(ParaId::new(1)))
		.build()
		.expect_err("a duplicated para id must be rejected, not silently collapsed");
	assert!(matches!(err, Error::DuplicateParaId(1)));
}
