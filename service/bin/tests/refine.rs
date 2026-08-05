//! `refine` entry-point tests, run against polkajam's in-memory node host.
//!
//! Blobs are embedded by the blob builder crates' build scripts, so `cargo test`
//! rebuilds them automatically when the guest sources change.

use codec::Encode;
use executor::{pj, pj::RefineOutcome};
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

pub const MOCK_CODE_HASH: ValidationCodeHash = ValidationCodeHash([123; 32]);

#[test]
fn trivial_works() {
	let config = good_config(1);
	let token = good_token();
	let auth_trace = good_trace();

	let validation_code_hash = ValidationCodeHash::from(SERVICE);
	let payload = ParachainCandidate { validation_code_hash, pov: Vec::new() }.encode();
	let work_items = vec![refine_work_item(SERVICE, payload, vec![Vec::new(), Vec::new()])];
	let (engine, code_hash, mut context) =
		refine_args(SERVICE, AUTHORIZER, config, token, auth_trace, work_items, 0);

	let outcome = pj::refine(&engine, code_hash, &mut context);
	expect_ok(outcome);
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
	let (engine, code_hash, mut context) =
		refine_args(SERVICE, AUTHORIZER, good_config(2), good_token(), good_trace(), work_items, 0);

	let outcome = pj::refine(&engine, code_hash, &mut context);
	assert_eq!(expect_log(outcome), RefineLog::InvalidItemCount);
}

#[test]
fn more_para_ids_than_work_items_errors() {
	let work_items = vec![refine_work_item(SERVICE, Vec::new(), vec![])];
	let (engine, code_hash, mut context) =
		refine_args(SERVICE, AUTHORIZER, good_config(2), good_token(), good_trace(), work_items, 0);

	let outcome = pj::refine(&engine, code_hash, &mut context);
	assert_eq!(expect_log(outcome), RefineLog::AuthConfigMismatch);
}

#[test]
fn less_para_ids_than_work_items_errors() {
	let work_items = vec![
		refine_work_item(SERVICE, Vec::new(), vec![]),
		refine_work_item(SERVICE, Vec::new(), vec![]),
	];
	let (engine, code_hash, mut context) =
		refine_args(SERVICE, AUTHORIZER, good_config(1), good_token(), good_trace(), work_items, 0);

	let outcome = pj::refine(&engine, code_hash, &mut context);
	assert_eq!(expect_log(outcome), RefineLog::InvalidItemCount);
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

fn expect_ok(res: anyhow::Result<RefineOutcome>) {
	let output = res.expect("Refine failed to return a ParachainWorkDigest");
	match output.digest {
		ParachainWorkDigest::Ok { .. } => (),
		ParachainWorkDigest::Err { error, .. } => panic!("RefineLog error: #{error:?}"),
	}
}
