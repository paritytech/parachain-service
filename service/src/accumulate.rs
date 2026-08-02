//! `accumulate` entry point of the parachain service.

use alloc::format;

use jam_pvm_common::accumulate::{accumulate_items, set_storage};
use jam_pvm_common::*;
use jam_types::{WorkOutput as WorkResult, *};

pub fn accumulate(slot: Slot, id: ServiceId, item_count: usize) -> Option<Hash> {
    // FIXME: I did not start with this. It is just dummy code here.
    info!(
        "This is Accumulate in the Parachain Service {id:x}h with {} items",
        item_count
    );

    for item in accumulate_items().into_iter() {
        match item {
            AccumulateItem::WorkItem(i) => on_accumulate_item(i),
            AccumulateItem::Transfer(t) => on_transfer(slot, t),
        }
    }
    None
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
