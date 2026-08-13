//! Incoming-transfer recording, consumption, and outbound replay (§5.1, D-6).

mod common;

use common::*;
use parachain_service::{
	constants::{MAX_INCOMING_TRANSFERS, MAX_TRANSFER_GAS},
	state::{
		log::{AccumulateLog, LogEntry},
		storage_key,
		transfers::IncomingTransferChain,
		Tag,
	},
	state_balance::INCOMING_TRANSFER_ENTRY_FOOTPRINT,
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
	let (_, storage, _) = run_block(ah_storage(), vec![transfer_item(9, 1_000_000)], NOW);

	let chain = transfer_chain(&storage).expect("chain created");
	assert_eq!(chain, IncomingTransferChain { first_slot: NOW, last_slot: NOW, count: 1 });
	let bucket = transfer_bucket(&storage, NOW).expect("bucket created");
	assert_eq!(bucket.transfers.len(), 1);
	assert_eq!(bucket.transfers[0].from, 9);
	assert_eq!(bucket.transfers[0].amount, 1_000_000);
	assert_eq!(bucket.next_slot, None);
}

#[test]
fn same_slot_bucket_append_works() {
	// Two arrivals in one slot share the bucket — no new storage item (§5.1).
	let items = vec![transfer_item(9, 1_000_000), transfer_item(10, 2_000_000)];
	let (_, storage, _) = run_block(ah_storage(), items, NOW);

	let chain = transfer_chain(&storage).unwrap();
	assert_eq!(chain.count, 2);
	assert_eq!(transfer_bucket(&storage, NOW).unwrap().transfers.len(), 2);
}

#[test]
fn chained_buckets_works() {
	// Arrivals in different slots link buckets through `next_slot`.
	let (_, storage, _) = run_block(ah_storage(), vec![transfer_item(9, 1_000_000)], NOW);
	let (_, storage, _) = run_block(storage, vec![transfer_item(10, 2_000_000)], NOW + 5);

	let chain = transfer_chain(&storage).unwrap();
	assert_eq!(chain, IncomingTransferChain { first_slot: NOW, last_slot: NOW + 5, count: 2 });
	assert_eq!(transfer_bucket(&storage, NOW).unwrap().next_slot, Some(NOW + 5));
	assert_eq!(transfer_bucket(&storage, NOW + 5).unwrap().next_slot, None);
}

#[test]
fn over_reserved_portion_admission_works() {
	// §5.1: past the reserved portion, only a transfer covering its own
	// worst-case slot cost is recorded.
	let full_chain =
		IncomingTransferChain { first_slot: 1, last_slot: 1, count: MAX_INCOMING_TRANSFERS as u32 };
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		set_state(s, &storage_key(Tag::IncomingTransferChain, &()), &full_chain);
		set_state(
			s,
			&storage_key(Tag::IncomingTransfers, &1u32),
			&parachain_service::state::transfers::IncomingTransfers {
				transfers: vec![],
				next_slot: None,
			},
		);
	});

	// Too small to pay for its own bucket: dropped without a trace.
	let small = INCOMING_TRANSFER_ENTRY_FOOTPRINT - 1;
	let (_, storage, _) = run_block(storage, vec![transfer_item(9, small)], NOW);
	assert_eq!(transfer_chain(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32);
	assert!(transfer_bucket(&storage, NOW).is_none());

	// Self-funding: recorded.
	let (_, storage, _) =
		run_block(storage, vec![transfer_item(9, INCOMING_TRANSFER_ENTRY_FOOTPRINT)], NOW + 1);
	assert_eq!(transfer_chain(&storage).unwrap().count, MAX_INCOMING_TRANSFERS as u32 + 1);
	assert!(transfer_bucket(&storage, NOW + 1).is_some());
}

#[test]
fn consume_works() {
	// Record at two slots, then Asset Hub consumes the first only.
	let (_, storage, _) = run_block(ah_storage(), vec![transfer_item(9, 1_000_000)], NOW);
	let (_, storage, _) = run_block(storage, vec![transfer_item(10, 2_000_000)], NOW + 5);

	let msg = UpwardMessage::ConsumeTransfersUpTo(NOW);
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW + 6);

	assert!(transfer_bucket(&storage, NOW).is_none());
	assert!(transfer_bucket(&storage, NOW + 5).is_some());
	assert_eq!(
		transfer_chain(&storage).unwrap(),
		IncomingTransferChain { first_slot: NOW + 5, last_slot: NOW + 5, count: 1 }
	);

	// Consuming the rest clears the chain entirely.
	let msg = UpwardMessage::ConsumeTransfersUpTo(NOW + 5);
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-1", b"ah-2", vec![msg], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW + 7);
	assert!(transfer_chain(&storage).is_none());
	assert!(transfer_bucket(&storage, NOW + 5).is_none());
}

#[test]
fn transfer_out_works() {
	// D-6: the destination's min_memo_gas is looked up at replay time.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, 500);
	});
	let msg = UpwardMessage::TransferOut { dest: 42, amount: 12345.into(), memo: [3; 128] };
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(mutations.transfers.len(), 1);
	let record = &mutations.transfers[0];
	assert_eq!(record.destination, 42);
	assert_eq!(record.amount, 12345);
	assert_eq!(record.gas_limit, 500);
	assert!(ah_accumulate_logs(&storage).is_empty());
}

#[test]
fn transfer_out_unknown_dest_errors() {
	let msg = UpwardMessage::TransferOut { dest: 999, amount: 12345.into(), memo: [3; 128] };
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(ah_storage(), vec![work_item(&digest)], NOW);

	assert!(mutations.transfers.is_empty());
	assert!(matches!(ah_accumulate_logs(&storage)[..], [AccumulateLog::TransferFailed { .. }]));
}

#[test]
fn transfer_out_gas_over_cap_errors() {
	// D-6: a destination demanding more than MAX_TRANSFER_GAS is never paid for.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, MAX_TRANSFER_GAS + 1);
	});
	let msg = UpwardMessage::TransferOut { dest: 42, amount: 12345.into(), memo: [3; 128] };
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(storage, vec![work_item(&digest)], NOW);

	assert!(mutations.transfers.is_empty());
	assert!(matches!(ah_accumulate_logs(&storage)[..], [AccumulateLog::TransferFailed { .. }]));
}
