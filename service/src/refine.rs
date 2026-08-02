//! `refine` entry point of the parachain service.

use alloc::vec::Vec;
use codec::Decode;
use jam_pvm_common::refine::{self, auth_trace};
use jam_types::{CoreIndex, ServiceId, WorkOutput as WorkResult, WorkPackageHash, WorkPayload};
use parachain_support::types::ParaId;

// The tuple payloads are only ever read through the `Debug` impl (they end up in
// the `error!` log emitted by `Service::refine`), which the dead-code lint does
// not count as a use.
#[allow(dead_code)]
#[derive(Debug)]
pub enum RefineError {
    InvalidAuthConfig,
    WrongParaIdCount(usize),
    WrongWorkItemCount(usize),
    WrongItemIndex(usize),
    WrongExtrinsicCount(u16),
}

pub fn refine(
    _core_index: CoreIndex,
    item_index: usize,
    _service_id: ServiceId,
    _payload: WorkPayload,
    _package_hash: WorkPackageHash,
) -> Result<WorkResult, RefineError> {
    let _auth_trace = auth_trace();
    let auth_config = refine::work_package().authorizer.config;

    let para_ids =
        Vec::<ParaId>::decode(&mut &auth_config[..]).map_err(|_| RefineError::InvalidAuthConfig)?;
    if para_ids.len() != 1 {
        return Err(RefineError::WrongParaIdCount(para_ids.len()));
    }

    let work_items = refine::work_items_summary();
    let [work_item]: &[_; 1] = work_items
        .as_slice()
        .try_into()
        .map_err(|_| RefineError::WrongWorkItemCount(work_items.len()))?;

    if item_index != 0 {
        return Err(RefineError::WrongItemIndex(item_index));
    }

    if work_item.extrinsics_count != 2 {
        return Err(RefineError::WrongExtrinsicCount(work_item.extrinsics_count));
    }

    // TODO: check if we should load them chunked to not OOM
    let _ext_para_state_proof = refine::extrinsic(0).expect("checked above");
    let _ext_jam_state_proof = refine::extrinsic(1).expect("checked above");

    Ok(Vec::new().into())
}
