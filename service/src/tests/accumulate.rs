//! `accumulate` entry-point tests.

use executor::jam::{self, AccumulateItem, WorkItemRecord, WorkOutput};

use super::service_blob;

// Traps in the guest's own accumulate logic on the empty `WorkItemRecord`.
// Un-ignore once this carries real inputs.
#[test]
#[ignore]
fn accumulate_runs() {
    let code = service_blob();
    let items = vec![AccumulateItem::WorkItem(WorkItemRecord {
        package: Default::default(),
        exports_root: Default::default(),
        authorizer_hash: Default::default(),
        payload: Default::default(),
        gas_limit: 0,
        result: Ok(WorkOutput(Vec::new())),
        auth_output: Default::default(),
    })];

    let outcome =
        jam::accumulate(&code, items).expect("accumulate should run to completion (not trap)");
    println!(
        "accumulate ok in {:?}, gas used {}, yielded: {:?}",
        outcome.elapsed, outcome.gas_used, outcome.yielded
    );
}
