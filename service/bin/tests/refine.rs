//! `refine` entry-point tests, run against polkajam's in-memory node host.
//!
//! Blobs are embedded by the blob builder crates' build scripts, so `cargo test`
//! rebuilds them automatically when the guest sources change.

use executor::pj;
use executor::pj::RefineOutcome;
use jam_types::{AuthConfig, AuthTrace, Authorization as AuthToken};
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service::work_digest::RefineLog;
use parachain_service_bin::mock::{good_config, good_token, refine_args, refine_work_item};
use parachain_service_bin::BLOB as SERVICE;

#[test]
fn trivial_works() {
    let config = good_config(1);
    let token = good_token();
    let auth_trace = AuthTrace::new(); // TODO hold author key

    // `refine` requires exactly two extrinsics (see `service/src/refine.rs`);
    // supply two empty placeholders so the call runs to completion.
    let work_items = vec![refine_work_item(
        SERVICE,
        Vec::new(),
        vec![Vec::new(), Vec::new()],
    )];
    let (engine, code_hash, mut context) = refine_args(
        SERVICE, AUTHORIZER, config, token, auth_trace, work_items, 0,
    );

    let outcome = pj::refine(&engine, code_hash, &mut context).unwrap();

    assert!(outcome.gas_used > 0, "refine should use some gas");
}

// Empty WPs are invalid per GP, hence panic.
#[test]
#[should_panic(expected = "the len is 0 but the index is 0")]
fn no_work_items_errors() {
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
    let (engine, code_hash, mut context) = refine_args(
        SERVICE,
        AUTHORIZER,
        AuthConfig::new(),
        AuthToken::new(),
        AuthTrace::new(),
        work_items,
        0,
    );

    let output = pj::refine(&engine, code_hash, &mut context);
    assert_eq!(expect_log(output), RefineLog::MalformedAuthorizerConfig);
}

/// Extract a RefineLog or panic.
fn expect_log(res: anyhow::Result<RefineOutcome>) -> RefineLog {
    let output = res.expect("Expected refine to return a ParachainWorkDigest");
    let log = output
        .digest
        .try_into_log()
        .expect("Expected refine to produce a RefineLog and not just `Ok`");
    log
}
