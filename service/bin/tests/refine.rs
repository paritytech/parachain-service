//! `refine` entry-point tests, run against polkajam's in-memory node host.
//!
//! Blobs are embedded by the blob builder crates' build scripts, so `cargo test`
//! rebuilds them automatically when the guest sources change.

use codec::{Decode, Encode};
use executor::{pj, pj::RefineOutcome};
use frameless::{hash_state, BlockData, Config, HeadData, MockAction, State, ValidationParams};
use jam_types::{AuthConfig, AuthTrace, Authorization as AuthToken};
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service::{
	refine::ParachainCandidate,
	work_digest::{ParachainWorkDigest, RefineLog, ValidationCodeHash},
};
use parachain_service_bin::{
	mock::{good_config, good_token, good_trace, refine_args, refine_work_item},
	BLOB as SERVICE,
};
use parachain_service_interface::types::{UpwardMessage, UpwardMessages};

pub const MOCK_CODE_HASH: ValidationCodeHash = ValidationCodeHash([123; 32]);

#[test]
fn trivial_works() {
	let config = good_config(1);
	let token = good_token();
	let auth_trace = good_trace();

	let pvf = frameless::WASM_BINARY.unwrap();
	let pvf_hash = ValidationCodeHash::from(pvf);

	// One block on top of the Coretime genesis: counter 0 -> 512.
	let parent = HeadData {
		number: 0,
		parent_hash: [0; 32],
		post_state: hash_state(&State { config: Config::Coretime, counter: 0 }),
	};
	let block = BlockData { state: State { config: Config::Coretime, counter: 0 }, add: 512 };
	let params = ValidationParams { parent_head: parent.encode(), block_data: block.encode() };

	let payload =
		ParachainCandidate { validation_code_hash: pvf_hash, pov: params.encode() }.encode();
	let work_items = vec![refine_work_item(SERVICE, payload, vec![Vec::new(), Vec::new()])];
	let (engine, code_hash, mut context) =
		refine_args(SERVICE, AUTHORIZER, config, token, auth_trace, work_items, &[pvf], 0);

	let outcome = pj::refine(&engine, code_hash, &mut context);

	// The inner PVM decoded the PoV, executed the block, and returned the new head data.
	let (head_data, upward_messages) = expect_ok(outcome);
	assert_eq!(upward_messages, UpwardMessages::new());

	let head = HeadData::decode(&mut &head_data[..]).expect("refine returned valid HeadData");
	assert_eq!(head.number, 1);
	assert_eq!(head.parent_hash, parent.hash());
	assert_eq!(head.post_state, hash_state(&State { config: Config::Coretime, counter: 512 }));
}

#[test]
fn send_upward_messages_works() {
	let config = good_config(1);
	let token = good_token();
	let auth_trace = good_trace();

	let pvf = frameless::WASM_BINARY.unwrap();
	let pvf_hash = ValidationCodeHash::from(pvf);

	let mock_action = MockAction::KVSet(b"KEY".to_vec(), b"VALUE".to_vec());
	let action = Config::Mock(vec![mock_action]);
	let parent = HeadData {
		number: 0,
		parent_hash: [0; 32],
		post_state: hash_state(&State { config: action.clone(), counter: 0 }),
	};
	let block = BlockData { state: State { config: action.clone(), counter: 0 }, add: 512 };
	let params = ValidationParams { parent_head: parent.encode(), block_data: block.encode() };

	let payload =
		ParachainCandidate { validation_code_hash: pvf_hash, pov: params.encode() }.encode();
	let work_items = vec![refine_work_item(SERVICE, payload, vec![Vec::new(), Vec::new()])];
	let (engine, code_hash, mut context) =
		refine_args(SERVICE, AUTHORIZER, config, token, auth_trace, work_items, &[pvf], 0);

	let outcome = pj::refine(&engine, code_hash, &mut context);

	// The inner PVM decoded the PoV, executed the block, and returned the new head data.
	let (head_data, upward_messages) = expect_ok(outcome);

	assert_eq!(
		upward_messages,
		UpwardMessages::try_from(vec![UpwardMessage::SetKV {
			key: b"KEY".to_vec(),
			value: b"VALUE".to_vec()
		},])
		.unwrap()
	);

	let head = HeadData::decode(&mut &head_data[..]).expect("refine returned valid HeadData");
	assert_eq!(head.number, 1);
	assert_eq!(head.parent_hash, parent.hash());
	assert_eq!(head.post_state, hash_state(&State { config: action, counter: 512 }));
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
	let log = output
		.digest
		.try_into_log()
		.expect("Expected refine to produce a RefineLog and not just `Ok`");
	log
}

fn expect_ok(res: anyhow::Result<RefineOutcome>) -> (Vec<u8>, UpwardMessages) {
	let output = res.expect("Refine failed to return a ParachainWorkDigest");
	match output.digest {
		ParachainWorkDigest::Ok { head_data, upward_messages, .. } => (head_data, upward_messages),
		ParachainWorkDigest::Err { error, .. } => panic!("RefineLog error: {error:?}"),
	}
}
