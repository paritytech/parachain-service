//! Service entry-point tests running against polkajam's in-memory node host.

use std::path::PathBuf;

use executor::jam::{self, AccumulateItem, WorkItemRecord, WorkOutput};

fn service_blob() -> Vec<u8> {
    blob("parachain-service.jam")
}

fn authorizer_blob() -> Vec<u8> {
    blob("parachain-authorizer.jam")
}

fn blob(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "reading {}: {error}\nBuild it first with `just build`.",
            path.display()
        )
    })
}

#[test]
fn refine_with_two_work_items_errors() {
    let code = service_blob();
    let authorizer = authorizer_blob();
    let work_items = vec![
        jam::work_item(&code, Vec::new()),
        jam::work_item(&code, Vec::new()),
    ];

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
fn refine_runs() {
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
