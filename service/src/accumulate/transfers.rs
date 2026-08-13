//! Incoming-transfer processing and outbound-transfer replay (spec §5.1).

use crate::{
	constants::{MAX_INCOMING_TRANSFERS, MAX_TRANSFER_GAS},
	hashing::blake2_256,
	state::{
		log::AccumulateLog,
		transfers::{
			IncomingTransferChain, IncomingTransfers, QueuedTransfer, TransferBuckets,
			TransferChain,
		},
	},
	state_balance::transfer_covers_own_slot,
};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::{service_info, transfer};
use jam_types::{Balance, Memo as JamMemo, ServiceId, Slot, TransferRecord};
use parachain_service_interface::types::{Memo, Timeslot};

/// §5.1 incoming-transfer processing. JAM credited the balance before this code
/// runs, so handling is best effort: within the pre-provisioned portion a
/// transfer is recorded unconditionally; beyond it only if its amount covers its
/// own worst-case queue entry. Otherwise it is dropped — no record, no log.
pub fn record_incoming(now: Slot, record: &TransferRecord) {
	let chain = TransferChain::get();
	let queued = chain.map_or(0, |c| c.count);
	let admitted =
		(queued as usize) < MAX_INCOMING_TRANSFERS || transfer_covers_own_slot(record.amount);
	if !admitted {
		return;
	}

	let transfer =
		QueuedTransfer { from: record.source, amount: record.amount, memo: record.memo.0 };

	match chain {
		None => {
			// Empty queue: `now` becomes the only bucket, so both endpoints.
			TransferBuckets::set(
				now,
				&IncomingTransfers { transfers: alloc::vec![transfer], next_slot: None },
			);
			TransferChain::set(&IncomingTransferChain {
				first_slot: now,
				last_slot: now,
				count: 1,
			});
		},
		Some(mut chain) if chain.last_slot == now => {
			// Same slot as the tail: append in place, no new storage item.
			let mut bucket = TransferBuckets::get(now).expect("chain names the tail; qed");
			bucket.transfers.push(transfer);
			TransferBuckets::set(now, &bucket);
			chain.count += 1;
			TransferChain::set(&chain);
		},
		Some(mut chain) => {
			// New bucket at `now`; link the old tail to it and move `last_slot`.
			let mut tail =
				TransferBuckets::get(chain.last_slot).expect("chain names the tail; qed");
			tail.next_slot = Some(now);
			TransferBuckets::set(chain.last_slot, &tail);
			TransferBuckets::set(
				now,
				&IncomingTransfers { transfers: alloc::vec![transfer], next_slot: None },
			);
			chain.last_slot = now;
			chain.count += 1;
			TransferChain::set(&chain);
		},
	}
}

/// §5.1 `consume_transfers_up_to(slot)`: drop whole buckets up to and including
/// `slot`, walking the chain from `first_slot` (Asset Hub only).
pub fn consume_up_to(slot: Timeslot) {
	let Some(mut chain) = TransferChain::get() else { return };
	let mut cursor = chain.first_slot;
	loop {
		if cursor > slot {
			chain.first_slot = cursor;
			TransferChain::set(&chain);
			return;
		}
		let bucket = TransferBuckets::get(cursor).expect("chain links only live buckets; qed");
		chain.count = chain.count.saturating_sub(bucket.transfers.len() as u32);
		TransferBuckets::remove(cursor);
		match bucket.next_slot {
			Some(next) => cursor = next,
			None => {
				// Chain exhausted.
				TransferChain::clear();
				return;
			},
		}
	}
}

/// Replay a `TransferOut` (Asset Hub only) via JAM `transfer` (D-6): the
/// destination's `min_memo_gas` is looked up at replay time and the call is
/// skipped (logged as `TransferFailed`) when it exceeds [`MAX_TRANSFER_GAS`] —
/// the sender's accumulate pays the transfer gas, so an uncapped value would
/// let a hostile destination burn this whole invocation's budget.
pub fn transfer_out(dest: ServiceId, amount: Balance, memo: &Memo, logs: &mut Vec<AccumulateLog>) {
	let failed = AccumulateLog::TransferFailed { memo_hash: blake2_256(memo) };
	let Some(min_memo_gas) = dest_min_memo_gas(dest) else {
		logs.push(failed);
		return;
	};
	if min_memo_gas > MAX_TRANSFER_GAS {
		logs.push(failed);
		return;
	}
	if transfer(dest, amount, min_memo_gas, &JamMemo(*memo)).is_err() {
		// WHO / LOW / CASH — only the memo hash is preserved (§5.1 step 7).
		logs.push(failed);
	}
}

fn dest_min_memo_gas(dest: ServiceId) -> Option<u64> {
	service_info(dest).map(|info| info.min_memo_gas)
}
