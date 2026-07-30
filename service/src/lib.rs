#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;
use accumulate::set_storage;
use alloc::{format, string::String};
use jam_pvm_common::*;
use jam_types::{WorkOutput as WorkResult, *};
use refine::export_slice;

pub struct ParachainService;
declare_service!(ParachainService);

impl Service for ParachainService {
    fn refine(
        _core_index: CoreIndex,
        _item_index: usize,
        _service_id: ServiceId,
        payload: WorkPayload,
        _package_hash: WorkPackageHash,
    ) -> WorkResult {
        let auth_trace = refine::auth_trace();
        let msg = format!(
            "Payload: <{}>, auth trace: <{}>",
            String::from_utf8_lossy(&payload),
            String::from_utf8_lossy(&auth_trace),
        );
        let export_index = export_slice(msg.as_bytes()).expect("infallible");
        (export_index, payload, auth_trace).encode().into()
    }

    fn accumulate(slot: Slot, id: ServiceId, item_count: usize) -> Option<Hash> {
        info!(
            "This is Accumulate in the Parachain Service {id:x}h with {} items",
            item_count
        );

        for item in accumulate::accumulate_items().into_iter() {
            match item {
                AccumulateItem::WorkItem(i) => on_accumulate_item(i),
                AccumulateItem::Transfer(t) => on_transfer(slot, t),
            }
        }
        None
    }
}

fn on_accumulate_item(item: WorkItemRecord) {
    let Ok(result) = item.result else { return };
    let root = item.exports_root;
    let (info_id, payload, auth_trace) =
        <(u64, WorkResult, AuthTrace)>::decode(&mut &result[..]).expect("infallible");
    let info = format!("0x{}... {info_id}", hex::to_hex(&root[..4]));
    set_storage(b"last-info", info.as_bytes()).expect("balance low");
    set_storage(b"last-payload", &payload).expect("balance low");
    set_storage(b"last-trace", &auth_trace).expect("balance low");
}

fn on_transfer(slot: Slot, item: TransferRecord) {
    let msg = format!(
        "Transfer at {slot} from {:x}h to {:x}h of {} memo {}",
        item.source, item.destination, item.amount, item.memo,
    );
    info!("{}", msg);
    set_storage(b"last-tx", msg.as_bytes()).expect("balance low");
}

#[cfg(test)]
mod tests;
