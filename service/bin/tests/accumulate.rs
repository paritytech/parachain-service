//! `accumulate` entry-point tests.

use codec::Encode;
use executor::pj;
use jam_types::{AccumulateItem, WorkItemRecord, WorkOutput};
use parachain_service::work_digest::{ParachainWorkDigest, ValidationCodeHash, ValidationCodeRef};
use parachain_service_bin::mock::accumulate_args;
use parachain_service_bin::BLOB as SERVICE;

// Traps in the guest's own accumulate logic on the empty `WorkItemRecord`.
// Un-ignore once this carries real inputs.
#[test]
fn trivial_works() {
    let digest = ParachainWorkDigest::Ok {
        para_id: 1000.into(),
        validation_code: ValidationCodeRef {
            hash: ValidationCodeHash([0; 32]),
            len: 123,
        },
        parent_head_hash: [1; 32],
        head_data: b"head data".into(),
        upward_messages: vec![],
        lookup_anchor: 456,
    };
    let items = vec![AccumulateItem::WorkItem(WorkItemRecord {
        package: Default::default(),
        exports_root: Default::default(),
        authorizer_hash: Default::default(),
        payload: Default::default(),
        gas_limit: 0,
        result: Ok(WorkOutput(digest.encode())),
        auth_output: Default::default(),
    })];

    let (engine, code_hash, mut context) = accumulate_args(SERVICE, items);
    let outcome = pj::accumulate(&engine, code_hash, &mut context)
        .expect("accumulate should run to completion (not trap)");
    assert!(outcome.gas_used > 0, "Must use some gas");
}
