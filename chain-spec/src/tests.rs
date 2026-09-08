//! Unit tests for the chain-spec builder.

use std::collections::{BTreeMap, BTreeSet};

use codec::{Decode, Encode};
use jam_types::{AuthorizerHash, Balance};
use parachain_authorizer::aura::AuthConfig;
use parachain_service::{
	state::{
		para_info::{ParaInfo, ValidationCode},
		preimage_registry::PreimageEntry,
		storage_key, Tag,
	},
	state_balance::{baseline_for, preimage_footprint},
	work_digest::{validation_code_hash, ValidationCodeRef},
};
use parachain_service_interface::types::{HeadData, ParaId, MAX_HEAD_DATA_SIZE};
use primitive_types::H256;

use crate::{parachain::ParachainSpec, Error, ParachainServiceSpec};

const SVC: u32 = 5;
const SERVICE_CODE: &[u8] = b"parachain service code";
const HEAD: &[u8] = b"heads up";
const CODE: &[u8] = b"validation code";
const CODE_2: &[u8] = b"other validation code";
const RICH: Balance = 10_000_000;

fn realm_config(para: ParaId) -> AuthConfig {
	AuthConfig {
		para_ids: vec![para],
		parachain_service: SVC,
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
fn para_info_entry(service: &jam_chainspec::GenesisService, para: ParaId) -> ParaInfo {
	decode(
		service
			.storage
			.get(&storage_key(Tag::Parachains, &para))
			.expect("para entry; qed"),
	)
}

#[test]
fn registered_para_layout() {
	let service = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(
			ParachainSpec::new(ParaId(3))
				.head_data(HEAD)
				.validation_code(CODE)
				.state_balance(RICH),
		)
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.id, SVC);
	assert_eq!(service.balance, Balance::MAX, "an unset balance keeps the unlimited default");
	assert_eq!(service.storage.len(), 2, "one para entry and one registry entry");

	// `[0x00] ‖ SCALE(ParaId)`, and the value round-trips to the expected `ParaInfo`.
	let key = storage_key(Tag::Parachains, &ParaId(3));
	assert_eq!(key, vec![0x00, 3, 0, 0, 0]);
	let info = para_info_entry(&service, ParaId(3));
	assert_eq!(info.head_data, HeadData::try_from(HEAD.to_vec()).expect("small head; qed"));
	assert_eq!(
		info.validation_code,
		Some(ValidationCode { code_ref: code_ref(CODE), pinned: false })
	);
	assert_eq!(info.pending_upgrade, None);
	assert_eq!(info.total_state_balance, RICH);
	assert_eq!(
		info.used_state_balance,
		baseline_for(ParaId(3)) + preimage_footprint(CODE.len() as u32)
	);
	assert!(!info.is_deregistering);
}

#[test]
fn validation_code_is_hosted_once_with_registry_entry() {
	let service = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(3)).validation_code(CODE))
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages, vec![CODE.to_vec()], "the code blob is hosted exactly once");
	let entry: PreimageEntry = decode(
		service
			.storage
			.get(&storage_key(Tag::PreimageRegistry, &(code_ref(CODE).hash.0, CODE.len() as u32)))
			.expect("registry entry; qed"),
	);
	assert_eq!(entry.referencers, BTreeSet::from([ParaId(3)]));
}

#[test]
fn shared_validation_code_is_hosted_once_and_referenced_by_both() {
	let service = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		// Out of order on purpose: output is ParaId-sorted.
		.parachain(ParachainSpec::new(ParaId(200)).validation_code(CODE))
		.parachain(ParachainSpec::new(ParaId(100)).validation_code(CODE))
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages.iter().filter(|blob| *blob == CODE).count(), 1);
	let entry: PreimageEntry = decode(
		service
			.storage
			.get(&storage_key(Tag::PreimageRegistry, &(code_ref(CODE).hash.0, CODE.len() as u32)))
			.expect("registry entry; qed"),
	);
	assert_eq!(entry.referencers, BTreeSet::from([ParaId(100), ParaId(200)]));
	for para in [ParaId(100), ParaId(200)] {
		let info = para_info_entry(&service, para);
		assert_eq!(
			info.validation_code,
			Some(ValidationCode { code_ref: code_ref(CODE), pinned: false })
		);
	}
}

#[test]
fn shared_authorizer_blob_is_hosted_once() {
	let verifier = b"authorizer blob".to_vec();
	let config = realm_config(ParaId(7));
	let service = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(7)).authorizer(verifier.clone(), &config))
		.parachain(ParachainSpec::new(ParaId(8)).authorizer(verifier.clone(), &config))
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages.iter().filter(|blob| **blob == verifier).count(), 1);
}

#[test]
fn authorizer_hashes_match_blake2b_concat() {
	let verifier = b"verifier blob".to_vec();
	let config = realm_config(ParaId(9));
	let spec = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(9)).authorizer(verifier.clone(), &config));

	// Independent computation, the raw contract: blake2b-256(code_hash ‖ SCALE(config)).
	let mut concat = blake2b(&verifier).to_vec();
	concat.extend_from_slice(&config.encode());

	let hashes = spec.authorizer_hashes();
	assert_eq!(hashes, BTreeMap::from([(ParaId(9), AuthorizerHash(blake2b(&concat)))]));
}

#[test]
fn oversized_head_data_is_a_typed_error() {
	let big = vec![0u8; MAX_HEAD_DATA_SIZE as usize + 1];
	let err = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(1)).head_data(big.clone()))
		.build()
		.expect_err("head over the bound must be rejected, not panicked");
	assert!(matches!(err, Error::HeadDataTooLarge { para: 1, len } if len == big.len()));

	// The bound itself fits.
	ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(1)).head_data(vec![0u8; MAX_HEAD_DATA_SIZE as usize]))
		.build()
		.expect("exactly 4 KiB fits; qed");
}

#[test]
fn para_without_validation_code_has_none_and_no_registry_entry() {
	let service = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(9)).state_balance(RICH))
		.build()
		.expect("a small spec builds; qed");

	let info = para_info_entry(&service, ParaId(9));
	assert_eq!(info.validation_code, None);
	assert_eq!(info.used_state_balance, baseline_for(ParaId(9)), "no preimage footprint");
	assert!(service.preimages.is_empty(), "nothing to host");
	assert!(
		service.storage.keys().all(|k| k[0] != Tag::PreimageRegistry as u8),
		"no registry entries for a para without a validation code"
	);
}

#[test]
fn extra_preimage_duplicate_of_validation_code_is_hosted_once() {
	let service = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(1)).validation_code(CODE))
		.preimage(CODE)
		.build()
		.expect("a small spec builds; qed");

	assert_eq!(service.preimages.iter().filter(|blob| *blob == CODE).count(), 1);
}

#[test]
fn build_is_deterministic_regardless_of_insertion_order() {
	let mk = |order: [ParaId; 3]| {
		let mut spec = ParachainServiceSpec::new(SVC, SERVICE_CODE);
		for id in order {
			let code: &[u8] = if id.0 % 2 == 0 { CODE } else { CODE_2 };
			spec = spec.parachain(ParachainSpec::new(id).validation_code(code).state_balance(RICH));
		}
		spec.build().expect("a small spec builds; qed")
	};

	let a = mk([ParaId(2), ParaId(1), ParaId(3)]);
	let b = mk([ParaId(3), ParaId(1), ParaId(2)]);
	assert_eq!(a.storage, b.storage);
	assert_eq!(a.preimages, b.preimages);
}

#[test]
fn duplicate_para_id_is_an_error() {
	let err = ParachainServiceSpec::new(SVC, SERVICE_CODE)
		.parachain(ParachainSpec::new(ParaId(1)))
		.parachain(ParachainSpec::new(ParaId(1)))
		.build()
		.expect_err("a duplicated para id must be rejected, not silently collapsed");
	assert!(matches!(err, Error::DuplicateParaId(1)));
}
