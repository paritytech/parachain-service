//! Incoming-transfer queue for Asset Hub (spec §3.1, §5.1): fixed-size buckets
//! keyed by an allocated [`BucketId`]. The ids are contiguous, so Asset Hub can
//! enumerate the queue from the two endpoints alone — JAM storage has no prefix
//! iteration, and contiguity is what replaces the old `next_slot` chaining.

use crate::{
	constants::MAX_TRANSFERS_PER_BUCKET,
	state::{self, StorageFull, Tag},
};
use bounded_collections::{BoundedVec, ConstU32};
use codec::{Compact, Decode, Encode};
use parachain_service_interface::types::{Balance, BucketId, Memo, ServiceId};

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct QueuedTransfer {
	pub from: ServiceId,
	#[codec(compact)]
	pub amount: Balance,
	pub memo: Memo,
}

/// One fixed-size bucket of the `incoming_transfers` queue, in arrival order.
pub type IncomingTransfers = BoundedVec<QueuedTransfer, ConstU32<MAX_TRANSFERS_PER_BUCKET>>;

/// Endpoints of the `incoming_transfers` queue. The occupied ids are exactly
/// `first_bucket ..= last_bucket`.
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
/// A missing entry is the design's empty queue, which is how ids restart at `0`.
pub struct TransferQueue;

impl TransferQueue {
	pub fn get() -> Option<IncomingTransferBuckets> {
		state::read_singleton(Tag::IncomingTransferBuckets)
	}

	/// Persist the endpoint pointer. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(buckets: &IncomingTransferBuckets) -> Result<(), StorageFull> {
		state::write_singleton(Tag::IncomingTransferBuckets, buckets)
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
