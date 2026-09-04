//! `refine` entry-point tests, run against polkajam's in-memory node host.
//!
//! Blobs are embedded by the blob builder crates' build scripts, so `cargo test`
//! rebuilds them automatically when the guest sources change.

use codec::{Decode, Encode};
use executor::{pj, pj::RefineOutcome};
use frameless::{
	blake2_256, hash_state, BlockData, Config, HeadData, MockAction, State, ValidationParams,
};
use jam_types::{AuthConfig, AuthTrace, Authorization as AuthToken, Hash};
use parachain_authorizer_ed25519_bin::BLOB as AUTHORIZER;
use parachain_service::{
	refine::ParachainCandidate,
	work_digest::{validation_code_hash, ParachainWorkDigest, RefineLog},
};
use parachain_service_bin::{
	mock::{good_config, good_config_for, good_token, good_trace, refine_args, refine_work_item},
	BLOB as SERVICE,
};
use parachain_service_interface::{
	types::{Balance, ParaId, ServiceId, ASSET_HUB_PARA_ID, CORETIME_PARA_ID},
	upward_message::{CreateServiceArgs, Target, TransferOutArgs, UpwardMessage, UpwardMessages},
};

/// A deferred `TransferOut` from this service's regular balance — the only shape
/// the vendored GP 0.7.2 host can execute (§5.1).
fn transfer_out_args(dest: ServiceId, amount: Balance) -> TransferOutArgs {
	TransferOutArgs {
		source: None,
		dest,
		amount: amount.into(),
		id: 1.into(),
		source_supervisor_balance: false,
		dest_supervisor_balance: false,
		deferred: Some(([9; 128], 500)),
	}
}

/// Run one frameless block (`counter += add`) built on `parent` through refine.
fn run_block(
	config: Config,
	parent: &HeadData,
	add: u64,
	para_ids: Vec<ParaId>,
) -> anyhow::Result<RefineOutcome> {
	let pvf = frameless::WASM_BINARY.unwrap();
	let pvf_hash = validation_code_hash(pvf);

	let block = BlockData { state: State { config, counter: 0 }, add };
	let params = ValidationParams { parent_head: parent.encode(), block_data: block.encode() };
	let payload =
		ParachainCandidate { validation_code_hash: pvf_hash, pov: params.encode() }.encode();
	let work_items = vec![refine_work_item(SERVICE, payload, vec![Vec::new(), Vec::new()])];
	let (engine, code_hash, mut context) = refine_args(
		SERVICE,
		AUTHORIZER,
		good_config_for(para_ids),
		good_token(),
		good_trace(),
		work_items,
		&[pvf],
		0,
	);
	pj::refine(&engine, code_hash, &mut context)
}

fn genesis(config: Config) -> HeadData {
	HeadData {
		number: 0,
		parent_hash: [0; 32],
		post_state: hash_state(&State { config, counter: 0 }),
	}
}

#[test]
fn trivial_works() {
	let parent = genesis(Config::Coretime);
	let outcome = run_block(Config::Coretime, &parent, 512, vec![ParaId(0)]);

	// The inner PVM decoded the PoV, executed the block, and declared the new
	// head + parent hash through host calls (D-1).
	let (parent_head_hash, head_data, upward_messages, lookup_anchor) = expect_ok(outcome);
	assert_eq!(upward_messages, UpwardMessages::new());
	assert_eq!(parent_head_hash, blake2_256(&parent.encode()));
	// The mock context's lookup-anchor slot.
	assert_eq!(lookup_anchor, 0);

	let head = HeadData::decode(&mut &head_data[..]).expect("refine returned valid HeadData");
	assert_eq!(head.number, 1);
	assert_eq!(head.parent_hash, parent.hash());
	assert_eq!(head.post_state, hash_state(&State { config: Config::Coretime, counter: 512 }));
}

#[test]
fn send_upward_messages_works() {
	let mock_action = MockAction::KVSet(b"KEY".to_vec(), b"VALUE".to_vec());
	let action = Config::Mock(vec![mock_action]);
	let parent = genesis(action.clone());

	let outcome = run_block(action.clone(), &parent, 512, vec![ParaId(0)]);

	let (_, head_data, upward_messages, _) = expect_ok(outcome);
	assert_eq!(
		upward_messages,
		UpwardMessages::try_from(vec![UpwardMessage::SetKV {
			key: b"KEY".to_vec(),
			value: b"VALUE".to_vec()
		}])
		.unwrap()
	);

	let head = HeadData::decode(&mut &head_data[..]).expect("refine returned valid HeadData");
	assert_eq!(head.number, 1);
	assert_eq!(head.post_state, hash_state(&State { config: action, counter: 512 }));
}

#[test]
fn report_error_works() {
	let action = Config::Mock(vec![MockAction::ReportError(b"complaint".to_vec())]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ParaId(0)]);

	let log = expect_log(outcome);
	assert_eq!(log, RefineLog::Opaque(b"complaint".to_vec().try_into().unwrap()));
}

#[test]
fn restricted_host_function_errors() {
	// `transfer_out` is Asset-Hub-only (§4.3); para 0 is not Asset Hub.
	let action = Config::Mock(vec![MockAction::TransferOut(transfer_out_args(42, 1))]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ParaId(0)]);

	assert_eq!(expect_log(outcome), RefineLog::RestrictedHostFunction);
}

#[test]
fn asset_hub_transfer_out_works() {
	// The same call is fine when the config binds the item to Asset Hub.
	let args = transfer_out_args(42, 1);
	let action = Config::Mock(vec![MockAction::TransferOut(args.clone())]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ASSET_HUB_PARA_ID]);

	let (_, _, upward_messages, _) = expect_ok(outcome);
	assert_eq!(upward_messages.into_iter().next(), Some(UpwardMessage::TransferOut(args)));
}

/// The §6.5 operations on a supervised service, paired with the upward message
/// each must produce once it clears the Asset-Hub gate.
fn service_op_actions() -> Vec<(MockAction, UpwardMessage)> {
	const VICTIM: ServiceId = 65_536;
	let create = CreateServiceArgs {
		code_hash: [7; 32],
		len: 1024.into(),
		min_item_gas: 0,
		min_memo_gas: 0,
		id: 77.into(),
		desired_id: Some(42),
		source_supervisor_balance: false,
		new_supervisor_balance: false,
	};
	vec![
		(
			MockAction::Forget { target: Target::Service(VICTIM), hash: [9; 32], len: 1024 },
			UpwardMessage::Forget {
				target: Target::Service(VICTIM),
				hash: [9; 32],
				len: 1024.into(),
			},
		),
		(
			MockAction::Solicit { target: Target::Service(VICTIM), hash: [9; 32], len: 1024 },
			UpwardMessage::Solicit {
				target: Target::Service(VICTIM),
				hash: [9; 32],
				len: 1024.into(),
			},
		),
		(
			MockAction::RemoveServiceStorage { service: VICTIM, key: vec![1, 2] },
			UpwardMessage::RemoveServiceStorage { service: VICTIM, key: vec![1, 2] },
		),
		(
			MockAction::EjectService { service: VICTIM },
			UpwardMessage::EjectService { service: VICTIM },
		),
		(
			MockAction::SetServiceSupervisor { service: VICTIM, new_supervisor: VICTIM },
			UpwardMessage::SetServiceSupervisor { service: VICTIM, new_supervisor: VICTIM },
		),
		(MockAction::CreateService(create.clone()), UpwardMessage::CreateService(create)),
	]
}

#[test]
fn service_ops_restricted_to_asset_hub_errors() {
	// §6.5: every supervised-service operation is Asset Hub only, so para 0
	// aborts the digest rather than emitting the message.
	for (action, _) in service_op_actions() {
		let config = Config::Mock(vec![action]);
		let parent = genesis(config.clone());

		let outcome = run_block(config, &parent, 1, vec![ParaId(0)]);

		assert_eq!(expect_log(outcome), RefineLog::RestrictedHostFunction);
	}
}

#[test]
fn service_ops_from_asset_hub_works() {
	// The same calls from Asset Hub round-trip through the child ABI unchanged.
	for (action, expected) in service_op_actions() {
		let config = Config::Mock(vec![action]);
		let parent = genesis(config.clone());

		let outcome = run_block(config, &parent, 1, vec![ASSET_HUB_PARA_ID]);

		let (_, _, upward_messages, _) = expect_ok(outcome);
		assert_eq!(upward_messages.into_iter().next(), Some(expected));
	}
}

#[test]
fn own_para_target_from_any_chain_works() {
	// §6.1: a `Parachain` target naming the caller stays unrestricted, so the
	// Asset-Hub gate above must not have caught the whole `forget`/`solicit` host
	// call — only its `Service`-targeted form.
	let cases = [
		(
			MockAction::Forget { target: Target::Parachain(ParaId(0)), hash: [9; 32], len: 8 },
			UpwardMessage::Forget {
				target: Target::Parachain(ParaId(0)),
				hash: [9; 32],
				len: 8.into(),
			},
		),
		(
			MockAction::Solicit { target: Target::Parachain(ParaId(0)), hash: [9; 32], len: 8 },
			UpwardMessage::Solicit {
				target: Target::Parachain(ParaId(0)),
				hash: [9; 32],
				len: 8.into(),
			},
		),
	];

	for (action, expected) in cases {
		let config = Config::Mock(vec![action]);
		let parent = genesis(config.clone());

		let outcome = run_block(config, &parent, 1, vec![ParaId(0)]);

		let (_, _, upward_messages, _) = expect_ok(outcome);
		assert_eq!(upward_messages.into_iter().next(), Some(expected));
	}
}

fn assign_action(len: usize, assigner: Option<u32>) -> Config {
	Config::Mock(vec![MockAction::AssignCore {
		core: 0,
		queue: vec![[3; 32]; len],
		assigner,
		jam_slot: 100,
	}])
}

#[test]
fn invalid_authorizer_queue_errors() {
	// TODO: Stale: Quint allows empty queues and short queues with a new assigner.
	for (len, assigner) in [(0, None), (81, None), (1, Some(7))] {
		let action = assign_action(len, assigner);
		let parent = genesis(action.clone());
		let outcome = run_block(action, &parent, 1, vec![CORETIME_PARA_ID]);
		assert_eq!(expect_log(outcome), RefineLog::InvalidAuthorizerQueue);
	}
}

#[test]
fn valid_authorizer_queue_works() {
	for (len, assigner) in [(1, None), (80, None), (80, Some(7))] {
		let action = assign_action(len, assigner);
		let parent = genesis(action.clone());
		let outcome = run_block(action, &parent, 1, vec![CORETIME_PARA_ID]);
		let (_, _, messages, _) = expect_ok(outcome);
		assert_eq!(messages.len(), 1);
	}
}

#[test]
fn repeated_validator_keys_errors() {
	let key = vec![0; 336];
	let action = Config::Mock(vec![
		MockAction::SetValidatorKeys { keys: key.clone(), is_last: false },
		MockAction::SetValidatorKeys { keys: key, is_last: true },
	]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ASSET_HUB_PARA_ID]);

	assert_eq!(expect_log(outcome), RefineLog::SetValidatorKeysRepeated);
}

#[test]
fn too_many_validator_keys_errors() {
	let action =
		Config::Mock(vec![MockAction::SetValidatorKeys { keys: vec![0; 31 * 336], is_last: true }]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ASSET_HUB_PARA_ID]);

	assert_eq!(expect_log(outcome), RefineLog::TooManyValidatorKeys);
}

#[test]
fn oversized_upward_messages_errors() {
	// Variant (1) + empty-key prefix (1) + four-byte value prefix (4) means
	// 40 KiB - 5 bytes of value is the first rejected encoding.
	let action = Config::Mock(vec![MockAction::KVSet(vec![], vec![0; 40 * 1024 - 5])]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ParaId(0)]);

	assert_eq!(expect_log(outcome), RefineLog::UpwardMessagesTooLarge);
}

#[test]
fn upward_messages_at_size_budget_works() {
	// The same encoding with one less value byte is exactly 40 KiB.
	let action = Config::Mock(vec![MockAction::KVSet(vec![], vec![0; 40 * 1024 - 6])]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ParaId(0)]);

	let (_, _, messages, _) = expect_ok(outcome);
	assert_eq!(messages.len(), 1);
}

#[test]
fn skip_head_declarations_errors() {
	let action = Config::Mock(vec![MockAction::SkipHeadDeclarations]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ParaId(0)]);

	assert_eq!(expect_log(outcome), RefineLog::MissingHeadDeclaration);
}

#[test]
fn duplicate_set_head_errors() {
	let action = Config::Mock(vec![MockAction::DuplicateSetHead(b"bogus".to_vec())]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ParaId(0)]);

	assert_eq!(expect_log(outcome), RefineLog::MissingHeadDeclaration);
}

// The authorizer is deployed as its own blob, so refine cannot assume the
// trace matches its compiled-in `aura::AuthTrace`. A foreign trace (here the
// sr25519 authorizer's `author_key ++ sudo` wire shape, one byte longer) must
// surface as a loggable digest for the named para, not trap the whole refine.
#[test]
fn foreign_auth_trace_errors() {
	let sr25519_era_trace = {
		let AuthTrace(mut raw) = good_trace();
		raw.push(0x00);
		AuthTrace(raw)
	};
	let work_items = vec![refine_work_item(SERVICE, Vec::new(), vec![])];
	let (engine, code_hash, mut context) = refine_args(
		SERVICE,
		AUTHORIZER,
		good_config(1),
		good_token(),
		sr25519_era_trace,
		work_items,
		&[],
		0,
	);

	let outcome = pj::refine(&engine, code_hash, &mut context);

	assert_eq!(expect_log(outcome), RefineLog::MalformedAuthTrace);
}

// Empty WPs are invalid per GP, hence panic.
#[test]
#[should_panic(expected = "the len is 0 but the index is 0")]
fn no_work_items_panicks() {
	let (engine, code_hash, mut context) = refine_args(
		SERVICE,
		AUTHORIZER,
		AuthConfig::new(),
		AuthToken::new(),
		AuthTrace::new(),
		Vec::new(),
		&[],
		0,
	);

	let _ = pj::refine(&engine, code_hash, &mut context);
}

// §4.1 step 1 / §4.2: every package-level failure panics, because none of them
// has an authoritative `para_id` to attribute a `RefineLog` to. The panic traps
// the PVM, so JAM sees a bare work error and there is no digest to log.

/// Assert that refine trapped rather than returning a digest.
fn expect_work_error(res: anyhow::Result<RefineOutcome>) {
	if let Ok(outcome) = res {
		panic!("expected a work error, got a digest: {:?}", outcome.digest);
	}
}

#[test]
fn two_work_items_errors() {
	let work_items = vec![
		refine_work_item(SERVICE, Vec::new(), vec![]),
		refine_work_item(SERVICE, Vec::new(), vec![]),
	];
	let (engine, code_hash, mut context) = refine_args(
		SERVICE,
		AUTHORIZER,
		good_config(2),
		good_token(),
		good_trace(),
		work_items,
		&[],
		0,
	);

	expect_work_error(pj::refine(&engine, code_hash, &mut context));
}

#[test]
fn more_para_ids_than_work_items_errors() {
	let work_items = vec![refine_work_item(SERVICE, Vec::new(), vec![])];
	let (engine, code_hash, mut context) = refine_args(
		SERVICE,
		AUTHORIZER,
		good_config(2),
		good_token(),
		good_trace(),
		work_items,
		&[],
		0,
	);

	expect_work_error(pj::refine(&engine, code_hash, &mut context));
}

#[test]
fn less_para_ids_than_work_items_errors() {
	let work_items = vec![
		refine_work_item(SERVICE, Vec::new(), vec![]),
		refine_work_item(SERVICE, Vec::new(), vec![]),
	];
	let (engine, code_hash, mut context) = refine_args(
		SERVICE,
		AUTHORIZER,
		good_config(1),
		good_token(),
		good_trace(),
		work_items,
		&[],
		0,
	);

	expect_work_error(pj::refine(&engine, code_hash, &mut context));
}

/// Extract a RefineLog or panic.
fn expect_log(res: anyhow::Result<RefineOutcome>) -> RefineLog {
	let output = res.expect("Refine failed to return a ParachainWorkDigest");
	output
		.digest
		.try_into_log()
		.expect("Expected refine to produce a RefineLog and not just `Ok`")
}

fn expect_ok(res: anyhow::Result<RefineOutcome>) -> (Hash, Vec<u8>, UpwardMessages, u32) {
	let output = res.expect("Refine failed to return a ParachainWorkDigest");
	match output.digest {
		ParachainWorkDigest::Ok {
			parent_head_hash,
			head_data,
			upward_messages,
			lookup_anchor,
			..
		} => (parent_head_hash, head_data.into_inner(), upward_messages, lookup_anchor),
		ParachainWorkDigest::Err { error, .. } => panic!("RefineLog error: {error:?}"),
	}
}
