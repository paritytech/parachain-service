//! `refine` entry point of the parachain service.

use alloc::{format, string::String};

use jam_pvm_common::refine::{self, auth_trace, export_slice};
use jam_pvm_common::*;
use jam_types::{WorkOutput as WorkResult, *};

pub fn refine(
    _core_index: CoreIndex,
    _item_index: usize,
    _service_id: ServiceId,
    payload: WorkPayload,
    _package_hash: WorkPackageHash,
) -> WorkResult {
    let auth_trace = auth_trace();

    let work_items = refine::work_items_summary();
    let [work_item]: &[_; 1] = work_items
        .as_slice()
        .try_into()
        .expect("there must be exactly one work item");

    if work_item.extrinsics_count != 2 {
        panic!("The work item needs exactly two extrinsics");
    }
    let ext_para_state_proof = refine::extrinsic(0).expect("checked above for 2 extrinsics; qed");
    let ext_jam_state_proof = refine::extrinsic(1).expect("checked above for 2 extrinsics; qed");

    Vec::new().into()
}
