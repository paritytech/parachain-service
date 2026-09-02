//! EXPERIMENT — runs both authorizer blobs through the PolkaJAM interpreter and prints the gas
//! one `is_authorized` call costs, so ed25519 and sr25519 can be compared. Passing also proves
//! the sr25519 guest verifies a real signature without trapping, which is the part a blob-size
//! measurement on its own would not tell you.
//!
//! Self-contained on purpose: `tools/executor` + `parachain-service-bin::mock`, the harness the
//! rest of the repo uses for this, does not compile against the `vendor/polkajam` commit this
//! branch is pinned to. See `../../README.md`.
//!
//! ```text
//! cargo test --release --offline \
//!     --manifest-path authorizer-sr25519-experiment/bin/Cargo.toml -- --nocapture
//! ```

use codec::Encode;
use jam_node::{
	vm::{Engine, Storage},
	PvmBackend,
};
use jam_std_common::{hash_raw, Service};
use jam_types::{
	AuthConfig as EncAuthConfig, Authorization as EncAuthToken, Authorizer, CodeHash,
	RefineContext, WorkItem, WorkPackage,
};
use primitive_types::H256;

const SERVICE_ID: jam_types::ServiceId = 0;
const SEED: [u8; 32] = [0x42; 32];

fn work_items() -> Vec<WorkItem> {
	vec![WorkItem {
		service: SERVICE_ID,
		code_hash: CodeHash::zero(),
		refine_gas_limit: 0,
		accumulate_gas_limit: 0,
		export_count: 0,
		payload: Default::default(),
		import_segments: Default::default(),
		extrinsics: Default::default(),
	}]
}

fn work_package(
	blob: &[u8],
	config: EncAuthConfig,
	authorization: EncAuthToken,
	items: Vec<WorkItem>,
) -> WorkPackage {
	WorkPackage {
		authorization,
		auth_code_host: SERVICE_ID,
		authorizer: Authorizer { code_hash: CodeHash(hash_raw(blob)), config },
		context: RefineContext {
			anchor: Default::default(),
			state_root: Default::default(),
			beefy_root: Default::default(),
			lookup_anchor: Default::default(),
			lookup_anchor_slot: 0,
			prerequisites: Default::default(),
		},
		items: items.try_into().expect("one work item fits the JAM bound"),
	}
}

fn storage_with_code(blob: &[u8]) -> Storage {
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
	storage.solicit(0, SERVICE_ID, code_hash, blob.len() as u32).expect("code fits");
	storage.provide(0, SERVICE_ID, blob).expect("code accepted");
	storage.commit();
	storage
}

fn run(blob: &[u8], config: EncAuthConfig, token: EncAuthToken) -> u64 {
	let package = work_package(blob, config, token, work_items());
	let storage = storage_with_code(blob);
	let engine = Engine::new(Some(PvmBackend::Interpreter)).expect("interpreter engine");
	let (result, gas_used) = engine.is_authorized(&package.into(), 0, &storage, None);
	result.expect("is_authorized should run to completion and authorize");
	gas_used
}

/// Build the config/token pair, given a way to produce a public key and a signature over the
/// signing payload. Both authorizers encode identical structures, so this is shared.
macro_rules! auth {
	($root:path, $blob:expr, $public:expr, $sign:expr) => {{
		use root::{aura, ParaId};
		use $root as root;
		let key: [u8; 32] = $public;
		let (root, proofs) = aura::build_collator_tree(&[key]);
		let config = aura::AuthConfig {
			para_ids: vec![ParaId(0)],
			parachain_service: SERVICE_ID,
			collator_set_root: root,
			collator_set_size: 1,
			slot_duration: 1,
		};
		let config_enc = EncAuthConfig(config.encode());
		let stub = EncAuthToken(
			aura::AuthToken {
				proof: vec![],
				key: [0; 32],
				signature: [0; 64],
				control_command: None,
			}
			.encode(),
		);
		let package = work_package($blob, config_enc.clone(), stub, work_items());
		let payload: H256 =
			aura::AuthToken::signing_payload(aura::signable_work_package_hash(&package), &None);
		let signature: [u8; 64] = $sign(payload);
		let token =
			aura::AuthToken { proof: proofs[0].clone(), key, signature, control_command: None };
		(config_enc, EncAuthToken(token.encode()))
	}};
}

#[test]
fn ed25519_vs_sr25519_gas() {
	let ed_blob = parachain_authorizer_ed25519_bin::BLOB;
	let signing_key = ed25519_dalek::SigningKey::from_bytes(&SEED);
	let (config, token) = {
		use ed25519_dalek::Signer;
		auth!(
			parachain_authorizer,
			ed_blob,
			signing_key.verifying_key().to_bytes(),
			|payload: H256| signing_key.sign(payload.as_bytes()).to_bytes()
		)
	};
	let ed_gas = run(ed_blob, config, token);

	let sr_blob = parachain_authorizer_sr25519_experiment_bin::BLOB;
	let keypair = schnorrkel::MiniSecretKey::from_bytes(&SEED)
		.expect("valid mini secret key")
		.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
	let (config, token) = auth!(
		parachain_authorizer_sr25519_experiment,
		sr_blob,
		keypair.public.to_bytes(),
		|payload: H256| keypair
			.sign_simple(
				parachain_authorizer_sr25519_experiment::aura::SIGNING_CONTEXT,
				payload.as_bytes()
			)
			.to_bytes()
	);
	let sr_gas = run(sr_blob, config, token);

	println!("ed25519: {} bytes, {} gas", ed_blob.len(), ed_gas);
	println!("sr25519: {} bytes, {} gas", sr_blob.len(), sr_gas);
}
