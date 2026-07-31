//! `refine` entry-point tests, run against polkajam's in-memory node host.
//!
//! Blobs are embedded by the wrapper crates' build scripts, so `cargo test`
//! rebuilds them automatically when the guest sources change.

use jam_types::{AuthConfig, AuthTrace, Authorization as AuthToken};
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::mock::{good_config, good_token};
use parachain_service_bin::BLOB as SERVICE;

#[test]
fn trivial_works() {
    let config = good_config(1);
    let token = good_token();
    let auth_trace = AuthTrace::new(); // TODO hold author key

    // `refine` requires exactly two extrinsics (see `service/src/refine.rs`);
    // supply two empty placeholders so the call runs to completion.
    let work_items = vec![executor::pj::work_item(
        SERVICE,
        Vec::new(),
        vec![Vec::new(), Vec::new()],
    )];

    let outcome = executor::pj::refine(
        SERVICE, AUTHORIZER, config, token, auth_trace, work_items, 0,
    )
    .expect("refine should run to completion (not trap)");

    assert!(outcome.gas_used > 0, "refine should use some gas");
}

#[test]
fn no_work_items_errors() {
    let work_items = vec![];

    executor::pj::refine(
        SERVICE,
        AUTHORIZER,
        AuthConfig::new(),
        AuthToken::new(),
        AuthTrace::new(),
        work_items,
        0,
    )
    .expect_err("empty WPs are forbidden by the Gray Paper; qed");
}

#[test]
fn two_work_items_errors() {
    let work_items = vec![
        executor::pj::work_item(SERVICE, Vec::new(), vec![]),
        executor::pj::work_item(SERVICE, Vec::new(), vec![]),
    ];

    executor::pj::refine(
        SERVICE,
        AUTHORIZER,
        AuthConfig::new(),
        AuthToken::new(),
        AuthTrace::new(),
        work_items,
        0,
    )
    .unwrap_err();
}
