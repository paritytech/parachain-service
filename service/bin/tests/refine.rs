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
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service::{
	refine::ParachainCandidate,
	work_digest::{validation_code_hash, ParachainWorkDigest, RefineLog},
};
use parachain_service_bin::{
	mock::{good_config, good_config_for, good_token, good_trace, refine_args, refine_work_item},
	BLOB as SERVICE,
};
use parachain_service_interface::{
	types::{ParaId, ASSET_HUB_PARA_ID},
	upward_message::{UpwardMessage, UpwardMessages},
};

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
	let action =
		Config::Mock(vec![MockAction::TransferOut { dest: 42, amount: 1, memo: [0; 128] }]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ParaId(0)]);

	assert_eq!(expect_log(outcome), RefineLog::RestrictedHostFunction);
}

#[test]
fn asset_hub_transfer_out_works() {
	// The same call is fine when the config binds the item to Asset Hub.
	let action =
		Config::Mock(vec![MockAction::TransferOut { dest: 42, amount: 1, memo: [9; 128] }]);
	let parent = genesis(action.clone());

	let outcome = run_block(action, &parent, 1, vec![ASSET_HUB_PARA_ID]);

	let (_, _, upward_messages, _) = expect_ok(outcome);
	assert_eq!(
		upward_messages.into_iter().next(),
		Some(UpwardMessage::TransferOut { dest: 42, amount: 1.into(), memo: [9; 128] })
	);
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

	let outcome = pj::refine(&engine, code_hash, &mut context);
	assert_eq!(expect_log(outcome), RefineLog::InvalidItemCount);
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

	let outcome = pj::refine(&engine, code_hash, &mut context);
	assert_eq!(expect_log(outcome), RefineLog::AuthConfigMismatch);
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

	let outcome = pj::refine(&engine, code_hash, &mut context);
	assert_eq!(expect_log(outcome), RefineLog::AuthConfigMismatch);
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
