//! Shared fixtures for the blob integration tests.

use codec::Encode;
use ed25519_dalek::{Signer, SigningKey};
use jam_node::{
	vm::{AccumulateCallContext, Engine, RefineCallContextOwned, StateMutations, Storage},
	PvmBackend,
};
use jam_std_common::{hash_raw, Entropy, Privileges, Service};
use jam_types::{
	AccumulateItem, AuthConfig, AuthTrace, Authorization as AuthToken, Authorizer, CodeHash,
	ExtrinsicHash, ExtrinsicSpec, FixedVec, ProtocolParameters, RefineContext, ServiceId, WorkItem,
	WorkPackage, WorkPayload,
};
use parachain_authorizer::{aura, ParaId};
use primitive_types::H256;

const SERVICE_ID: ServiceId = 0;
const GAS_LIMIT: u64 = 5_000_000_000;

/// Fixed ed25519 seed for deterministic test collator key generation.
const COLLATOR_SEED: [u8; 32] = [0x42; 32];

pub type RefineWorkItem = (WorkItem, Vec<Vec<u8>>);

fn blake2b_32(data: &[u8]) -> [u8; 32] {
	let mut out = [0u8; 32];
	out.copy_from_slice(blake2b_simd::Params::new().hash_length(32).hash(data).as_bytes());
	out
}

/// Blake2b-32 leaf hash for a collator public key (mirrors the authorizer's check_proof).
pub fn collator_leaf_hash(key: &[u8; 32]) -> H256 {
	H256::from_slice(&blake2b_32(key))
}

/// Build a binary Merkle tree over a set of collator public-key bytes.
///
/// Returns `(root, proofs)` where `proofs[i]` is the Vec<H256> of sibling hashes
/// for collator `i` (leaf-to-root, LSB-first — matching check_proof in aura.rs).
/// The tree is zero-hash-padded to the next power of two.
pub fn build_collator_tree(keys: &[[u8; 32]]) -> (H256, Vec<Vec<H256>>) {
	assert!(!keys.is_empty());
	let n = keys.len();
	let size = n.next_power_of_two();
	let zero = [0u8; 32];

	let mut levels: Vec<Vec<[u8; 32]>> = Vec::new();
	let mut leaf_level: Vec<[u8; 32]> = keys.iter().map(|k| blake2b_32(k)).collect();
	while leaf_level.len() < size {
		leaf_level.push(zero);
	}
	levels.push(leaf_level);

	while levels.last().unwrap().len() > 1 {
		let prev = levels.last().unwrap().clone();
		let next: Vec<[u8; 32]> = prev
			.chunks(2)
			.map(|c| {
				let mut input = [0u8; 64];
				input[..32].copy_from_slice(&c[0]);
				input[32..].copy_from_slice(&c[1]);
				blake2b_32(&input)
			})
			.collect();
		levels.push(next);
	}

	let root = H256::from_slice(&levels.last().unwrap()[0]);

	let mut proofs: Vec<Vec<H256>> = Vec::new();
	for idx in 0..n {
		let mut proof: Vec<H256> = Vec::new();
		let mut i = idx;
		for level in &levels[..levels.len() - 1] {
			let sibling_idx = i ^ 1;
			let sib = level.get(sibling_idx).copied().unwrap_or(zero);
			proof.push(H256::from_slice(&sib));
			i >>= 1;
		}
		proofs.push(proof);
	}

	(root, proofs)
}

/// An authorizer config whose `ParaId` prefix authorizes `para_ids` packages,
/// numbering the paras `0..n`.
pub fn good_config(para_ids: usize) -> AuthConfig {
	let ids = (0..para_ids).map(|i| ParaId(i as u32)).collect::<Vec<_>>();
	good_config_for(ids)
}

/// An authorizer config binding the given `ParaId`s to the package's work items.
///
/// Uses the fixed `COLLATOR_SEED` to derive a real collator key and build a
/// real single-collator Merkle root. The signature in `good_token()` is a stub
/// ([0;64]); use `make_auth` for happy-path tests that run is_authorized.
pub fn good_config_for(para_ids: Vec<ParaId>) -> AuthConfig {
	let signing_key = SigningKey::from_bytes(&COLLATOR_SEED);
	let key_bytes: [u8; 32] = signing_key.verifying_key().to_bytes();
	let (root, _) = build_collator_tree(&[key_bytes]);
	let config = aura::AuthConfig {
		para_ids,
		parachain_service: SERVICE_ID,
		collator_set_root: root,
		collator_set_size: 1,
		slot_duration: 1,
	};
	AuthConfig(config.encode())
}

/// A well-formed AURA authorization token carrying the fixed collator key and
/// an empty Merkle proof (valid for a 1-collator tree), but a stub zero-valued
/// signature. Use for error-path tests only — the signature will fail
/// `verify_strict`. Use `make_auth` for happy-path tests.
pub fn good_token() -> AuthToken {
	let signing_key = SigningKey::from_bytes(&COLLATOR_SEED);
	let key_bytes: [u8; 32] = signing_key.verifying_key().to_bytes();
	let token = aura::AuthToken { proof: vec![], key: key_bytes, signature: [0u8; 64] };
	AuthToken(token.encode())
}

pub fn good_trace() -> AuthTrace {
	let trace = aura::AuthTrace { author_key: [0; 32], sudo: false };
	AuthTrace(trace.encode())
}

/// Build a real `(AuthConfig, AuthToken, AuthTrace)` triple using `COLLATOR_SEED`.
///
/// Computes the signable WorkPackage hash and signs it with the collator's key,
/// so `is_authorized` will accept the returned args when called through the PVM.
pub fn make_auth(
	authorizer_blob: &[u8],
	para_ids: Vec<ParaId>,
	items: &[WorkItem],
) -> (AuthConfig, AuthToken, AuthTrace) {
	make_auth_with_seed(authorizer_blob, para_ids, items, COLLATOR_SEED)
}

/// Like [`make_auth`] but using the given `seed` for the collator signing key.
///
/// Useful for RFC 8032-known-answer tests or other fixed-seed scenarios.
pub fn make_auth_with_seed(
	authorizer_blob: &[u8],
	para_ids: Vec<ParaId>,
	items: &[WorkItem],
	seed: [u8; 32],
) -> (AuthConfig, AuthToken, AuthTrace) {
	let signing_key = SigningKey::from_bytes(&seed);
	let key_bytes: [u8; 32] = signing_key.verifying_key().to_bytes();

	let (root, proofs) = build_collator_tree(&[key_bytes]);
	let proof = proofs[0].clone();

	let config = aura::AuthConfig {
		para_ids,
		parachain_service: SERVICE_ID,
		collator_set_root: root,
		collator_set_size: 1,
		slot_duration: 1,
	};
	let config_enc = AuthConfig(config.encode());

	let dummy_token =
		AuthToken(aura::AuthToken { proof: vec![], key: [0u8; 32], signature: [0u8; 64] }.encode());
	let pkg = work_package(authorizer_blob, config_enc.clone(), dummy_token, items.to_vec());
	let wp_hash = aura::signable_work_package_hash(&pkg);

	let sig_bytes: [u8; 64] = signing_key.sign(wp_hash.as_bytes()).to_bytes();

	let token = aura::AuthToken { proof, key: key_bytes, signature: sig_bytes };
	let trace = aura::AuthTrace { author_key: key_bytes, sudo: false };

	(config_enc, AuthToken(token.encode()), AuthTrace(trace.encode()))
}

/// Build `is_authorized_args` for a single-collator tree keyed by `key`,
/// with the given `signature`. Used to test bad-key and weak-key rejection.
///
/// The Merkle proof is empty (single-collator tree, root = leaf hash of `key`).
pub fn make_single_collator_args_with_key(
	authorizer_blob: &[u8],
	para_ids: Vec<ParaId>,
	items: Vec<WorkItem>,
	key: [u8; 32],
	signature: [u8; 64],
) -> (Engine, WorkPackage, Storage) {
	let root = collator_leaf_hash(&key);
	let config = aura::AuthConfig {
		para_ids,
		parachain_service: SERVICE_ID,
		collator_set_root: root,
		collator_set_size: 1,
		slot_duration: 1,
	};
	let token = aura::AuthToken { proof: vec![], key, signature };
	is_authorized_args(
		authorizer_blob,
		AuthConfig(config.encode()),
		AuthToken(token.encode()),
		items,
	)
}

/// Build `is_authorized_args` for a multi-collator tree where the token carries
/// the proof for `proof_for_collator` but the authorizer expects a different
/// collator (derived from the slot). Used to test proof-for-wrong-index rejection.
///
/// All collator keys are derived from `all_seeds` (one per seed). The token is
/// signed by `signing_seed`'s key and carries `all_seeds[proof_for_collator]`'s
/// proof. The signature is zero-valued since the proof check fails first.
pub fn make_wrong_collator_index_args(
	authorizer_blob: &[u8],
	para_ids: Vec<ParaId>,
	items: Vec<WorkItem>,
	all_seeds: &[[u8; 32]],
	proof_for_collator: usize,
) -> (Engine, WorkPackage, Storage) {
	assert!(all_seeds.len() >= 2);
	let keys: Vec<[u8; 32]> = all_seeds
		.iter()
		.map(|s| SigningKey::from_bytes(s).verifying_key().to_bytes())
		.collect();
	let (root, proofs) = build_collator_tree(&keys);
	let key = keys[proof_for_collator];
	let proof = proofs[proof_for_collator].clone();

	let config = aura::AuthConfig {
		para_ids,
		parachain_service: SERVICE_ID,
		collator_set_root: root,
		collator_set_size: all_seeds.len() as u32,
		slot_duration: 1,
	};
	let token = aura::AuthToken { proof, key, signature: [0u8; 64] };
	is_authorized_args(
		authorizer_blob,
		AuthConfig(config.encode()),
		AuthToken(token.encode()),
		items,
	)
}

/// `n` minimal work items addressed to the parachain service.
pub fn work_items(n: usize) -> Vec<WorkItem> {
	(0..n)
		.map(|_| WorkItem {
			service: SERVICE_ID,
			code_hash: CodeHash::zero(),
			refine_gas_limit: 0,
			accumulate_gas_limit: 0,
			export_count: 0,
			payload: Default::default(),
			import_segments: Default::default(),
			extrinsics: Default::default(),
		})
		.collect()
}

pub fn refine_work_item(
	service_blob: &[u8],
	payload: Vec<u8>,
	extrinsics: Vec<Vec<u8>>,
) -> RefineWorkItem {
	let specs = extrinsics
		.iter()
		.map(|bytes| ExtrinsicSpec {
			hash: ExtrinsicHash(hash_raw(bytes)),
			len: bytes.len() as u32,
		})
		.collect::<Vec<_>>()
		.try_into()
		.expect("extrinsic count exceeds the JAM bound");

	let item = WorkItem {
		service: SERVICE_ID,
		code_hash: CodeHash(hash_raw(service_blob)),
		refine_gas_limit: GAS_LIMIT,
		accumulate_gas_limit: 0,
		export_count: 0,
		payload: WorkPayload(payload),
		import_segments: Default::default(),
		extrinsics: specs,
	};

	(item, extrinsics)
}

pub fn refine_args(
	service_blob: &[u8],
	authorizer_blob: &[u8],
	config: AuthConfig,
	token: AuthToken,
	auth_trace: AuthTrace,
	work_items: Vec<RefineWorkItem>,
	preimages: &[&[u8]],
	work_item_index: usize,
) -> (Engine, CodeHash, RefineCallContextOwned) {
	let (items, extrinsic_data): (Vec<_>, Vec<_>) = work_items.into_iter().unzip();
	let package = work_package(authorizer_blob, config, token, items);
	let (mut storage, code_hash) = storage_with_code(service_blob);
	for blob in preimages {
		provide_preimage(&mut storage, blob);
	}

	let context = RefineCallContextOwned {
		storage,
		core: 0,
		service_id: SERVICE_ID,
		gas: GAS_LIMIT,
		auth_output: auth_trace,
		import_data: Vec::new(),
		extrinsic_data: extrinsic_data
			.into_iter()
			.map(|per_item| per_item.into_iter().map(Into::into).collect())
			.collect(),
		export_counter: 0,
		max_exports: 0,
		exports: Vec::new(),
		work_package: package.into(),
		work_item_index,
		output_len: 0,
		engine: engine(),
	};

	(engine(), code_hash, context)
}

pub fn is_authorized_args(
	authorizer_blob: &[u8],
	config: AuthConfig,
	token: AuthToken,
	work_items: Vec<WorkItem>,
) -> (Engine, WorkPackage, Storage) {
	relax_authorizer_code_limit(authorizer_blob.len());
	let package = work_package(authorizer_blob, config, token, work_items);
	let (storage, _) = storage_with_code(authorizer_blob);
	(engine(), package, storage)
}

fn relax_authorizer_code_limit(blob_len: usize) {
	if jam_types::max_authorizer_code_size() < blob_len {
		let mut p = ProtocolParameters::get();
		p.max_authorizer_code_size = blob_len as u32;
		p.apply().expect("relaxed authorizer code limit is valid");
	}
}

pub fn accumulate_args(
	service_blob: &[u8],
	items: Vec<AccumulateItem>,
) -> (Engine, CodeHash, AccumulateCallContext<'static>) {
	accumulate_args_at(service_blob, items, 0, |_| {})
}

/// [`accumulate_args`] at a given `slot`, with `seed` applied to the storage
/// first (e.g. `set_service_key` for genesis service state, or `provide` for
/// preimages).
pub fn accumulate_args_at(
	service_blob: &[u8],
	items: Vec<AccumulateItem>,
	slot: u32,
	seed: impl FnOnce(&mut Storage),
) -> (Engine, CodeHash, AccumulateCallContext<'static>) {
	let (mut storage, code_hash) = storage_with_code(service_blob);
	seed(&mut storage);
	storage.commit();
	(engine(), code_hash, accumulate_context(storage, items, slot))
}

/// An accumulate call context over an existing storage — for sequential-block
/// tests that thread the storage from a previous run's context.
pub fn accumulate_context(
	storage: Storage,
	items: Vec<AccumulateItem>,
	slot: u32,
) -> AccumulateCallContext<'static> {
	accumulate_context_with_privileges(
		storage,
		items,
		slot,
		Privileges {
			bless: SERVICE_ID,
			assign: FixedVec::new(SERVICE_ID),
			designate: SERVICE_ID,
			register: SERVICE_ID,
			always_acc: Default::default(),
		},
	)
}

/// [`accumulate_context`] with explicit JAM `Privileges` — lets tests override
/// e.g. the `designate`/`assign`/`bless` services to exercise negative
/// privilege paths.
pub fn accumulate_context_with_privileges(
	storage: Storage,
	items: Vec<AccumulateItem>,
	slot: u32,
	privileges: jam_std_common::Privileges,
) -> AccumulateCallContext<'static> {
	AccumulateCallContext {
		storage,
		mutations: StateMutations::new(0),
		snapshot: None,
		gas: GAS_LIMIT,
		slot,
		service_id: SERVICE_ID,
		entropy: Entropy::default(),
		items,
		privileges,
		cost: None,
	}
}

/// The mock's own service id, for tests that need to reference it.
pub const MOCK_SERVICE_ID: ServiceId = SERVICE_ID;

pub fn work_package(
	authorizer_blob: &[u8],
	config: AuthConfig,
	authorization: AuthToken,
	work_items: Vec<WorkItem>,
) -> WorkPackage {
	WorkPackage {
		authorization,
		auth_code_host: SERVICE_ID,
		authorizer: Authorizer { code_hash: CodeHash(hash_raw(authorizer_blob)), config },
		context: RefineContext {
			anchor: Default::default(),
			state_root: Default::default(),
			beefy_root: Default::default(),
			lookup_anchor: Default::default(),
			lookup_anchor_slot: 0,
			prerequisites: Default::default(),
		},
		items: work_items.try_into().expect("work-item count exceeds the JAM bound"),
	}
}

/// Put preimage into store for historical_lookup to resolve.
pub fn provide_preimage(storage: &mut Storage, blob: &[u8]) {
	let hash = hash_raw(blob);
	storage
		.solicit(0, SERVICE_ID, hash, blob.len() as u32)
		.expect("preimage should fit in storage");
	storage
		.provide(0, SERVICE_ID, blob)
		.expect("preimage should be accepted by storage");
	storage.commit();
}

fn storage_with_code(blob: &[u8]) -> (Storage, CodeHash) {
	let code_hash = hash_raw(blob);
	let service = Service {
		code_hash: CodeHash(code_hash),
		balance: 1_000_000_000_000,
		min_item_gas: 100,
		min_memo_gas: 100,
		bytes: 1_000_000,
		items: 1_000,
		deposit_offset: 0,
		creation_slot: 0,
		last_accumulation_slot: 0,
		parent_service: 0,
	};
	let mut storage = Storage::new_empty();
	storage.set_service(SERVICE_ID, &service);
	storage
		.solicit(0, SERVICE_ID, code_hash, blob.len() as u32)
		.expect("service code should fit in storage");
	storage
		.provide(0, SERVICE_ID, blob)
		.expect("service code should be accepted by storage");
	storage.commit();
	(storage, CodeHash(code_hash))
}

fn engine() -> Engine {
	Engine::new(Some(PvmBackend::Interpreter)).expect("interpreter engine should initialize")
}
