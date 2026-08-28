//! Incoming-transfer queue for Asset Hub (spec §3.1, §5.1): buckets keyed by
//! arrival timeslot, chained through `next_slot` because JAM storage has no
//! prefix iteration.

use crate::state::{self, StorageFull, Tag};
use alloc::vec::Vec;
use codec::{Compact, Decode, Encode};
use parachain_service_interface::types::{Balance, Memo, ServiceId, Timeslot};

/// One recorded incoming transfer.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct QueuedTransfer {
	pub from: ServiceId,
	#[codec(compact)]
	pub amount: Balance,
	pub memo: Memo,
}

/// One slot's bucket in the `incoming_transfers` chain.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct IncomingTransfers {
	/// Transfers that arrived in this slot, in arrival order.
	pub transfers: Vec<QueuedTransfer>,
	/// Next occupied slot, `None` at the tail.
	pub next_slot: Option<Timeslot>,
}

/// Endpoints of the `incoming_transfers` chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct IncomingTransferChain {
	pub first_slot: Timeslot,
	pub last_slot: Timeslot,
	/// Total transfers held across every bucket. The §5.1 admission rule counts
	/// transfers, but JAM storage has no prefix iteration, so the count cannot
	/// be recovered by scanning buckets.
	pub count: u32,
}

// Compile-time check that the compact-encoded amount derives from `u64` (D-3).
const _: fn(Balance) -> Compact<u64> = Compact::<u64>;

/// Storage accessors for the `incoming_transfers` map (tag `0x06`).
pub struct TransferBuckets;

impl TransferBuckets {
	pub fn get(slot: Timeslot) -> Option<IncomingTransfers> {
		state::read(Tag::IncomingTransfers, &slot)
	}

	/// Upsert a bucket. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(slot: Timeslot, bucket: &IncomingTransfers) -> Result<(), StorageFull> {
		state::write(Tag::IncomingTransfers, &slot, bucket)
	}

	pub fn remove(slot: Timeslot) {
		state::clear(Tag::IncomingTransfers, &slot)
	}
}

/// Storage accessors for the `incoming_transfer_chain` singleton (tag `0x07`).
/// A missing entry is the design's `None`.
pub struct TransferChain;

impl TransferChain {
	pub fn get() -> Option<IncomingTransferChain> {
		state::read_singleton(Tag::IncomingTransferChain)
	}

	/// Persist the chain pointer. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(chain: &IncomingTransferChain) -> Result<(), StorageFull> {
		state::write_singleton(Tag::IncomingTransferChain, chain)
	}

	pub fn clear() {
		state::clear_singleton(Tag::IncomingTransferChain)
	}
}
