//! Incoming-transfer recording, consumption, and outbound replay (§5.1, D-6).

mod common;

use common::*;
use parachain_service::{
	constants::{MAX_INCOMING_TRANSFERS, MAX_TRANSFER_GAS},
	state::{
		log::{AccumulateLog, LogEntry, TransferError},
		storage_key,
		transfers::{IncomingTransferBuckets, IncomingTransfers, QueuedTransfer},
		Tag,
	},
	state_balance::{excess_transfer_footprint, INCOMING_TRANSFER_ENTRY_FOOTPRINT},
};
use parachain_service_interface::{types::ASSET_HUB_PARA_ID, upward_message::UpwardMessage};

const NOW: u32 = 100;
const AH_CODE: &[u8] = b"ah-code";

fn ah_storage() -> jam_node::vm::Storage {
	fresh_storage(|s| seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH))
}

fn ah_accumulate_logs(storage: &jam_node::vm::Storage) -> Vec<AccumulateLog> {
	para_log(storage, ASSET_HUB_PARA_ID)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect()
}

#[test]
fn record_works() {
	let (_, storage, _) = accumulate_block(ah_storage(), vec![transfer_item(9, 1_000_000)], NOW);

	let queue = transfer_queue(&storage).expect("queue created");
	assert_eq!(queue, IncomingTransferBuckets { first_bucket: 0, last_bucket: 0, count: 1 });
	let bucket = transfer_bucket(&storage, 0).expect("bucket created");
	assert_eq!(bucket.transfers.len(), 1);
	assert_eq!(bucket.transfers[0].from, 9);
	assert_eq!(bucket.transfers[0].amount, 1_000_000);
}

#[test]
fn one_invocation_shares_bucket_works() {
	// Two arrivals in one invocation share the bucket (§5.1).
	let items = vec![transfer_item(9, 1_000_000), transfer_item(10, 2_000_000)];
	let (_, storage, _) = accumulate_block(ah_storage(), items, NOW);

	let queue = transfer_queue(&storage).unwrap();
	assert_eq!(queue.count, 2);
	assert_eq!(transfer_bucket(&storage, 0).unwrap().transfers.len(), 2);
}

#[test]
fn invocation_boundary_closes_bucket_works() {
	// A later invocation never reopens the preceding invocation's bucket.
	let (_, storage, _) = accumulate_block(ah_storage(), vec![transfer_item(9, 1_000_000)], NOW);
	let (_, storage, _) = accumulate_block(storage, vec![transfer_item(10, 2_000_000)], NOW + 5);

	let queue = transfer_queue(&storage).unwrap();
	assert_eq!(queue, IncomingTransferBuckets { first_bucket: 0, last_bucket: 1, count: 2 });
	assert_eq!(transfer_bucket(&storage, 0).unwrap().transfers.len(), 1);
	assert_eq!(transfer_bucket(&storage, 1).unwrap().transfers.len(), 1);
}

#[test]
fn over_reserved_portion_admission_works() {
	// §5.1: past the reserved portion, only a transfer covering its own
	// worst-case slot cost is recorded.
	let full_queue =
		IncomingTransferBuckets { first_bucket: 0, last_bucket: 0, count: MAX_INCOMING_TRANSFERS as u32 };
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		set_state(s, &storage_key(Tag::IncomingTransferBuckets, &()), &full_queue);
		set_state(
			s,
			&storage_key(Tag::IncomingTransfers, &0u64),
			&parachain_service::state::transfers::IncomingTransfers {
				transfers: vec![],
			},
		);
	});

	// Too small to pay for its own bucket: dropped without a trace.
	let small = INCOMING_TRANSFER_ENTRY_FOOTPRINT - 1;
	let (_, storage, _) = accumulate_block(storage, vec![transfer_item(9, small)], NOW);
	assert_eq!(transfer_queue(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32);
	assert!(transfer_bucket(&storage, 1).is_none());

	// Self-funding: recorded.
	let (_, storage, _) = accumulate_block(
		storage,
		vec![transfer_item(9, INCOMING_TRANSFER_ENTRY_FOOTPRINT)],
		NOW + 1,
	);
	assert_eq!(transfer_queue(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32 + 1);
	assert!(transfer_bucket(&storage, 1).is_some());
}

#[test]
fn reservation_edge_admission_works() {
	// §5.1: one short of the reservation the entry is still inside it, so a
	// zero-amount transfer is free and fills the reservation exactly. Only from
	// `MAX_INCOMING_TRANSFERS` onwards must an entry pay for itself.
	let storage = chain_storage(MAX_INCOMING_TRANSFERS as u32 - 1);
	let (_, storage, _) = accumulate_block(storage, vec![transfer_item(9, 0)], NOW);

	assert_eq!(transfer_queue(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32);
	assert!(transfer_bucket(&storage, 1).is_some());

	// The next one is past the reservation: a zero-amount transfer is dropped.
	let (_, storage, _) = accumulate_block(storage, vec![transfer_item(9, 0)], NOW + 1);
	assert_eq!(transfer_queue(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32);
	assert!(transfer_bucket(&storage, 2).is_none());
}

#[test]
fn clean_up_buckets_works() {
	// Record in two invocations, then Asset Hub cleans up the first bucket only.
	let (_, storage, _) = accumulate_block(ah_storage(), vec![transfer_item(9, 1_000_000)], NOW);
	let (_, storage, _) = accumulate_block(storage, vec![transfer_item(10, 2_000_000)], NOW + 5);

	let msg = UpwardMessage::CleanUpBucketsUpTo(0);
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], NOW + 6);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 6);

	assert!(transfer_bucket(&storage, 0).is_none());
	assert!(transfer_bucket(&storage, 1).is_some());
	assert_eq!(
		transfer_queue(&storage).unwrap(),
		IncomingTransferBuckets { first_bucket: 1, last_bucket: 1, count: 1 }
	);

	// Cleaning up the rest clears the queue entirely.
	let msg = UpwardMessage::CleanUpBucketsUpTo(1);
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-1", b"ah-2", vec![msg], NOW + 7);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 7);
	assert!(transfer_queue(&storage).is_none());
	assert!(transfer_bucket(&storage, 1).is_none());

	// An emptied queue restarts allocation at bucket zero.
	let (_, storage, _) = accumulate_block(storage, vec![transfer_item(11, 3_000_000)], NOW + 8);
	assert_eq!(transfer_queue(&storage).unwrap().first_bucket, 0);
	assert!(transfer_bucket(&storage, 0).is_some());
}

#[test]
fn bucket_spills_at_capacity_works() {
	let items = (0..parachain_service::constants::MAX_TRANSFERS_PER_BUCKET + 1)
		.map(|i| transfer_item(i as u32, 1_000_000))
		.collect();
	let (_, storage, _) = accumulate_block(ah_storage(), items, NOW);
	assert_eq!(transfer_bucket(&storage, 0).unwrap().transfers.len(), 512);
	assert_eq!(transfer_bucket(&storage, 1).unwrap().transfers.len(), 1);
	assert_eq!(transfer_queue(&storage).unwrap().last_bucket, 1);
}

/// A queue claiming `count` queued transfers in one bucket.
fn chain_storage(count: u32) -> jam_node::vm::Storage {
	fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		set_state(
			s,
			&storage_key(Tag::IncomingTransferBuckets, &()),
			&IncomingTransferBuckets { first_bucket: 0, last_bucket: 0, count },
		);
		set_state(
			s,
			&storage_key(Tag::IncomingTransfers, &0u64),
			&IncomingTransfers {
				transfers: vec![QueuedTransfer { from: 0, amount: 0, memo: [0; 128] }],
			},
		);
	})
}

/// A chain holding exactly the reservation, so the next entry must self-fund.
fn full_chain_storage() -> jam_node::vm::Storage {
	chain_storage(MAX_INCOMING_TRANSFERS as u32)
}

#[test]
fn unreserved_transfers_are_self_funded_works() {
	// §5.1: draining refunds exactly what admission charged, so an unreserved
	// entry cannot ratchet `used_state_balance` upward over time. Both `used`
	// and `total` move by the per-bucket cost, never by the transfer's `amount`.
	let storage = full_chain_storage();
	let used0 = para_info(&storage, ASSET_HUB_PARA_ID).unwrap().used_state_balance;
	let total0 = para_info(&storage, ASSET_HUB_PARA_ID).unwrap().total_state_balance;

	// Deliberately far more than one entry costs, to pin that the surplus is
	// not credited to the allowance.
	let at = 2;
	let (_, storage, _) = accumulate_block(
		storage,
		vec![transfer_item(3, INCOMING_TRANSFER_ENTRY_FOOTPRINT * 100)],
		at,
	);
	let pi = para_info(&storage, ASSET_HUB_PARA_ID).unwrap();
	assert_eq!(transfer_queue(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32 + 1);
	assert_eq!(pi.used_state_balance, used0 + INCOMING_TRANSFER_ENTRY_FOOTPRINT);
	assert_eq!(pi.total_state_balance, total0 + INCOMING_TRANSFER_ENTRY_FOOTPRINT);

	// Drain the whole queue back out through the real accumulate path.
	let msg = UpwardMessage::CleanUpBucketsUpTo(1);
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], at);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], at + 1);

	assert!(transfer_queue(&storage).is_none());
	let pi = para_info(&storage, ASSET_HUB_PARA_ID).unwrap();
	assert_eq!(pi.used_state_balance, used0);
	assert_eq!(pi.total_state_balance, total0);
}

#[test]
fn draining_unreserved_restores_balance_works() {
	// §5.1: a full queue plus three self-funded entries, then the three oldest
	// drained on the next round. The queue is full again and the state balance
	// is back to its pre-charge value.
	let storage = full_chain_storage();
	let used0 = para_info(&storage, ASSET_HUB_PARA_ID).unwrap().used_state_balance;
	let total0 = para_info(&storage, ASSET_HUB_PARA_ID).unwrap().total_state_balance;
	let big = INCOMING_TRANSFER_ENTRY_FOOTPRINT;

	// Push three entries, each in its own fresh slot past the tail.
	let mut storage = storage;
	let mut now = 2;
	for _ in 0..3 {
		let (_, s, _) = accumulate_block(storage, vec![transfer_item(3, big * 100)], now);
		storage = s;
		now += 1;
	}
	let pi = para_info(&storage, ASSET_HUB_PARA_ID).unwrap();
	assert_eq!(transfer_queue(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32 + 3);
	assert_eq!(pi.used_state_balance, used0 + 3 * big);
	assert_eq!(pi.total_state_balance, total0 + 3 * big);

	// Clean up the seed bucket plus two arrivals: three
	// transfers gone, the queue returns to the reservation size and the charge
	// is refunded.
	let msg = UpwardMessage::CleanUpBucketsUpTo(2);
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 3);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], now);

	let pi = para_info(&storage, ASSET_HUB_PARA_ID).unwrap();
	assert_eq!(transfer_queue(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32);
	assert_eq!(pi.used_state_balance, used0);
	assert_eq!(pi.total_state_balance, total0);
}

#[test]
fn excess_transfer_footprint_works() {
	// §5.1: the reservation itself costs nothing beyond the baseline.
	assert_eq!(excess_transfer_footprint(MAX_INCOMING_TRANSFERS as u64), 0);
	assert_eq!(
		excess_transfer_footprint(MAX_INCOMING_TRANSFERS as u64 + 3),
		3 * INCOMING_TRANSFER_ENTRY_FOOTPRINT
	);
}

#[test]
fn transfer_out_works() {
	// §5.1: a deferred transfer forwards the caller's own gas limit.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, 500);
	});
	let msg = transfer_out_msg(42, 12345, 7, Some(([3; 128], 500)));
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(mutations.transfers.len(), 1);
	let record = &mutations.transfers[0];
	assert_eq!(record.destination, 42);
	assert_eq!(record.amount, 12345);
	assert_eq!(record.gas_limit, 500);
	assert!(ah_accumulate_logs(&storage).is_empty());
}

#[test]
fn transfer_out_below_dest_minimum_errors() {
	// §5.1: the caller now supplies the gas, so under-funding is its own error.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, 500);
	});
	let msg = transfer_out_msg(42, 12345, 8, Some(([3; 128], 499)));
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	assert!(mutations.transfers.is_empty());
	assert_eq!(
		ah_accumulate_logs(&storage),
		vec![AccumulateLog::TransferFailed {
			id: 8.into(),
			error: TransferError::GasBelowDestinationMinimum
		}]
	);
}

#[test]
fn transfer_out_plain_move_errors() {
	// §5.1: a plain move needs supervision of `dest`, which this service lacks.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, 500);
	});
	let msg = transfer_out_msg(42, 12345, 9, None);
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	assert!(mutations.transfers.is_empty());
	assert_eq!(
		ah_accumulate_logs(&storage),
		vec![AccumulateLog::TransferFailed {
			id: 9.into(),
			error: TransferError::DestinationNotSupervised
		}]
	);
}

#[test]
fn transfer_out_unknown_dest_errors() {
	let msg = transfer_out_msg(999, 12345, 3, Some(([3; 128], 500)));
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = accumulate_block(ah_storage(), vec![work_item(&digest)], NOW);

	assert!(mutations.transfers.is_empty());
	assert_eq!(
		ah_accumulate_logs(&storage),
		vec![AccumulateLog::TransferFailed {
			id: 3.into(),
			error: TransferError::UnknownDestination
		}]
	);
}

#[test]
fn transfer_out_gas_over_cap_errors() {
	// D-6: forwarded gas above MAX_TRANSFER_GAS is never committed, since
	// `Ω_T` charges it to this service's own accumulate meter.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, MAX_TRANSFER_GAS + 1);
	});
	let msg = transfer_out_msg(42, 12345, 4, Some(([3; 128], MAX_TRANSFER_GAS + 1)));
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	assert!(mutations.transfers.is_empty());
	assert_eq!(
		ah_accumulate_logs(&storage),
		vec![AccumulateLog::TransferFailed {
			id: 4.into(),
			error: TransferError::InsufficientServiceBalance
		}]
	);
}
