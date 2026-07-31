//! `refine` entry-point tests, run against polkajam's in-memory node host.
//!
//! Blobs are embedded by the wrapper crates' build scripts, so `cargo test`
//! rebuilds them automatically when the guest sources change.

use codec::Encode;
use executor::jam;

use parachain_authorizer::ParaId;
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::BLOB as SERVICE;

#[test]
fn trivial_works() {
    // Run the authorizer first, exactly as a guarantor would (spec: `Ψ_I` before
    // `Ψ_R`), and feed its auth trace into refine as the `r` argument. The same
    // config/token go into refine so its work package matches what was authorized.
    let config = jam::AuthConfig(vec![ParaId(1)].encode());
    let token = jam::AuthToken::new();
    let auth_trace = jam::is_authorized(AUTHORIZER, config.clone(), token.clone(), 0)
        .expect("is_authorized should run to completion (not trap)")
        .auth_trace;

    // `refine` requires exactly two extrinsics (see `service/src/refine.rs`);
    // supply two empty placeholders so the call runs to completion.
    let work_items = vec![jam::work_item(SERVICE, Vec::new(), vec![Vec::new(), Vec::new()])];

    let outcome = jam::refine(SERVICE, AUTHORIZER, config, token, auth_trace, work_items, 0)
        .expect("refine should run to completion (not trap)");

    assert!(outcome.gas_used > 0, "refine should use some gas");
}

#[test]
fn no_work_items_errors() {
    let work_items = vec![];

    jam::refine(
        SERVICE,
        AUTHORIZER,
        jam::AuthConfig::new(),
        jam::AuthToken::new(),
        jam::AuthTrace::new(),
        work_items,
        0,
    )
    .expect_err("empty WPs are forbidden by the Gray Paper; qed");
}

#[test]
fn two_work_items_errors() {
    let work_items = vec![
        jam::work_item(SERVICE, Vec::new(), vec![]),
        jam::work_item(SERVICE, Vec::new(), vec![]),
    ];

    jam::refine(
        SERVICE,
        AUTHORIZER,
        jam::AuthConfig::new(),
        jam::AuthToken::new(),
        jam::AuthTrace::new(),
        work_items,
        0,
    )
    .unwrap_err();
}
