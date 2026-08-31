//! Incoming-transfer queue for Asset Hub (spec §3.1, §5.1): buckets keyed by
//! contiguous service-allocated bucket ids.

use crate::state::{self, StorageFull, Tag};
use alloc::vec::Vec;
use codec::{Compact, Decode, Encode};
use parachain_service_interface::{
	types::{Balance, Memo, ServiceId},
	upward_message::BucketId,
};

/// One recorded incoming transfer.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct QueuedTransfer {
	pub from: ServiceId,
	#[codec(compact)]
	pub amount: Balance,
	pub memo: Memo,
}

/// One fixed-size bucket in the `incoming_transfers` queue.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct IncomingTransfers {
	pub transfers: Vec<QueuedTransfer>,
}

/// Endpoints of the contiguous `incoming_transfers` queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct IncomingTransferBuckets {
	pub first_bucket: BucketId,
	pub last_bucket: BucketId,
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
	pub fn get(id: BucketId) -> Option<IncomingTransfers> {
		state::read(Tag::IncomingTransfers, &id)
	}

	/// Upsert a bucket. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(id: BucketId, bucket: &IncomingTransfers) -> Result<(), StorageFull> {
		state::write(Tag::IncomingTransfers, &id, bucket)
	}

	pub fn remove(id: BucketId) {
		state::clear(Tag::IncomingTransfers, &id)
	}
}

/// Storage accessors for the `incoming_transfer_buckets` singleton (tag `0x07`).
/// A missing entry is the design's `None`.
pub struct TransferQueue;

impl TransferQueue {
	pub fn get() -> Option<IncomingTransferBuckets> {
		state::read_singleton(Tag::IncomingTransferBuckets)
	}

	/// Persist the chain pointer. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(queue: &IncomingTransferBuckets) -> Result<(), StorageFull> {
		state::write_singleton(Tag::IncomingTransferBuckets, queue)
	}

	pub fn clear() {
		state::clear_singleton(Tag::IncomingTransferBuckets)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn endpoint_encoding_works() {
		let endpoints = IncomingTransferBuckets { first_bucket: 1, last_bucket: 2, count: 3 };
		assert_eq!(endpoints.encode().len(), 8 + 8 + 4);
		assert_eq!(Some(endpoints).encode().len(), 1 + 8 + 8 + 4);
	}
}
