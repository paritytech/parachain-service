//! `refine` entry-point tests, run against polkajam's in-memory node host.
//!
//! Blobs are embedded by the wrapper crates' build scripts, so `cargo test`
//! rebuilds them automatically when the guest sources change.

use executor::jam;

use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::BLOB as SERVICE;

#[test]
fn trivial_works() {
    let work_items = vec![jam::work_item(SERVICE, Vec::new())];

    let outcome = jam::refine(SERVICE, AUTHORIZER, work_items, 0)
        .expect("refine should run to completion (not trap)");

    assert!(outcome.gas_used > 0, "refine should use some gas");
}

#[test]
fn no_work_items_errors() {
    let work_items = vec![];

    jam::refine(SERVICE, AUTHORIZER, work_items, 0)
        .expect_err("empty WPs are forbidden by the Gray Paper; qed");
}

#[test]
fn two_work_items_errors() {
    let work_items = vec![
        jam::work_item(SERVICE, Vec::new()),
        jam::work_item(SERVICE, Vec::new()),
    ];

    jam::refine(SERVICE, AUTHORIZER, work_items, 0).unwrap_err();
}
