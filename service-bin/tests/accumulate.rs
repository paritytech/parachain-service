//! `accumulate` entry-point tests.

use parachain_service_bin::BLOB as SERVICE;
use jam_types::{AccumulateItem, WorkItemRecord, WorkOutput};

// Traps in the guest's own accumulate logic on the empty `WorkItemRecord`.
// Un-ignore once this carries real inputs.
#[test]
#[ignore]
fn accumulate_runs() {
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
        executor::pj::accumulate(SERVICE, items).expect("accumulate should run to completion (not trap)");
    println!(
        "accumulate ok in {:?}, gas used {}, yielded: {:?}",
        outcome.elapsed, outcome.gas_used, outcome.yielded
    );
}
