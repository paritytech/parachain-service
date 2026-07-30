//! `refine` entry-point tests.

use executor::jam;

use super::{authorizer_blob, service_blob};

#[test]
fn trivial_works() {
    let code = service_blob();
    let authorizer = authorizer_blob();
    let work_items = vec![jam::work_item(&code, Vec::new())];

    let outcome = jam::refine(&code, &authorizer, work_items, 0)
        .expect("refine should run to completion (not trap)");
    println!(
        "refine ok in {:?}, gas used {}, output: {} bytes",
        outcome.elapsed,
        outcome.gas_used,
        outcome.output.len()
    );
}

#[test]
fn no_work_items_errors() {
    let code = service_blob();
    let authorizer = authorizer_blob();
    let work_items = vec![];

    jam::refine(&code, &authorizer, work_items, 0).expect_err("empty WPs are forbidden by the Gray Paper; qed");
}

#[test]
fn two_work_items_errors() {
    let code = service_blob();
    let authorizer = authorizer_blob();
    let work_items = vec![
        jam::work_item(&code, Vec::new()),
        jam::work_item(&code, Vec::new()),
    ];

    jam::refine(&code, &authorizer, work_items, 0).unwrap_err();
}
