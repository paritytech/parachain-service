//! A mock JAM transfer-destination service for gas benchmarks.
//!
//! Stands in for a legitimate foreign destination of a `TransferOut` (e.g. an
//! exchange deposit service) to size `MAX_TRANSFER_GAS`: its memo handler does
//! the realistic minimum of bookkeeping — index the transfer in a forward and
//! a backward lookup map and bump a received-counter. Not part of the
//! parachain service; only embedded by `parachain-service-bin`'s `test-utils`.

#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;

use jam_pvm_common::{
	accumulate::{accumulate_items, get_storage, set_storage},
	declare_service, Service,
};
use jam_types::{
	AccumulateItem, CoreIndex, Hash, ServiceId, Slot, TransferRecord, WorkOutput, WorkPackageHash,
	WorkPayload,
};

/// Directory of this crate's `Cargo.toml`, used by `parachain-service-bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

const COUNT_KEY: &[u8] = b"c";

pub struct MockDestService;
declare_service!(MockDestService);

impl Service for MockDestService {
	fn refine(
		_core_index: CoreIndex,
		_item_index: usize,
		_service_id: ServiceId,
		_payload: WorkPayload,
		_package_hash: WorkPackageHash,
	) -> WorkOutput {
		// The mock only ever receives transfers.
		WorkOutput(Default::default())
	}

	fn accumulate(_slot: Slot, _id: ServiceId, _item_count: usize) -> Option<Hash> {
		for item in accumulate_items() {
			if let AccumulateItem::Transfer(transfer) = item {
				handle_transfer(&transfer);
			}
		}
		None
	}
}

/// The realistic memo handler: the first 32 memo bytes are a sender-chosen
/// deposit reference, indexed both ways, plus one counter increment. The
/// counter is read-modify-written per transfer — each transfer pays what it
/// would cost arriving alone.
fn handle_transfer(transfer: &TransferRecord) {
	let reference = &transfer.memo.0[..32];

	// Forward map: reference -> (sender, amount).
	let mut fwd_key = [0u8; 33];
	fwd_key[0] = b'f';
	fwd_key[1..].copy_from_slice(reference);
	let mut fwd_val = [0u8; 12];
	fwd_val[..4].copy_from_slice(&transfer.source.to_le_bytes());
	fwd_val[4..].copy_from_slice(&transfer.amount.to_le_bytes());
	set_storage(&fwd_key, &fwd_val).expect("mock destination is generously funded; qed");

	// Backward map: sender -> reference.
	let mut bwd_key = [0u8; 5];
	bwd_key[0] = b'b';
	bwd_key[1..].copy_from_slice(&transfer.source.to_le_bytes());
	set_storage(&bwd_key, reference).expect("mock destination is generously funded; qed");

	// Counter increment.
	let count = get_storage(COUNT_KEY).map_or(0u64, |raw| {
		u64::from_le_bytes(raw[..8].try_into().expect("counter is 8 bytes; qed"))
	});
	set_storage(COUNT_KEY, &(count + 1).to_le_bytes())
		.expect("mock destination is generously funded; qed");
}
