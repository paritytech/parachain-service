//! Incoming-transfer processing and outbound-transfer replay (spec §5.1).

use crate::{
	constants::{MAX_INCOMING_TRANSFERS, MAX_TRANSFER_GAS},
	state::{
		log::{AccumulateLog, InsufficientBalanceReason, TransferError},
		transfers::{
			IncomingTransferChain, IncomingTransfers, QueuedTransfer, TransferBuckets,
			TransferChain,
		},
	},
	state_balance::{reattribute_transfer_queue, transfer_covers_own_slot},
};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::{service_info, transfer};
use jam_types::{Memo as JamMemo, Slot, TransferRecord};
use parachain_service_interface::{types::Timeslot, upward_message::TransferOutArgs};

/// §5.1 incoming-transfer processing. JAM credited the balances before this
/// code runs, so handling is best effort: within the pre-provisioned portion a
/// transfer is recorded unconditionally; beyond it only if its amount covers
/// its own worst-case queue entry. Otherwise it is dropped — no record, no log.
///
/// All of one block's operands arrive at the same `now`, so the admitted
/// transfers land in a single bucket write. Per-transfer writes would re-read
/// and re-write the growing bucket each time — measured at 55x the `Ga` budget
/// for 1024 same-slot transfers (D-8); the resulting state is identical.
///
/// Returns the `InsufficientStateBalance` entries for bucket/chain writes that
/// hit the §6.1 backstop; the caller routes them to Asset Hub's
/// parachain log.
pub fn record_incoming(now: Slot, records: &[&TransferRecord]) -> Vec<AccumulateLog> {
	let mut logs = Vec::new();
	let mut chain = TransferChain::get();
	let mut queued = chain.as_ref().map_or(0, |c| c.count);
	let old_count = queued;
	let mut admitted: Vec<QueuedTransfer> = Vec::new();
	for record in records {
		// §5.1: inside the reservation the entry is already paid for.
		if (queued as usize) < MAX_INCOMING_TRANSFERS || transfer_covers_own_slot(record.amount) {
			queued += 1;
			admitted.push(QueuedTransfer {
				from: record.source,
				amount: record.amount,
				memo: record.memo.0,
			});
		}
	}
	if admitted.is_empty() {
		return logs;
	}
	let added = admitted.len() as u32;
	let mut reject = || {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::IncomingTransfer,
		});
	};

	match &mut chain {
		None => {
			// Empty queue: `now` becomes the only bucket, so both endpoints.
			if TransferBuckets::set(
				now,
				&IncomingTransfers { transfers: admitted, next_slot: None },
			)
			.is_err() || TransferChain::set(&IncomingTransferChain {
				first_slot: now,
				last_slot: now,
				count: added,
			})
			.is_err()
			{
				reject();
			}
		},
		Some(chain) if chain.last_slot == now => {
			// Same slot as the tail: append in place, no new storage item.
			let mut bucket = TransferBuckets::get(now).expect("chain names the tail; qed");
			bucket.transfers.extend(admitted);
			if TransferBuckets::set(now, &bucket).is_err() {
				reject();
			}
			chain.count += added;
			if TransferChain::set(chain).is_err() {
				reject();
			}
		},
		Some(chain) => {
			// New bucket at `now`; link the old tail to it and move `last_slot`.
			let mut tail =
				TransferBuckets::get(chain.last_slot).expect("chain names the tail; qed");
			tail.next_slot = Some(now);
			if TransferBuckets::set(chain.last_slot, &tail).is_err() ||
				TransferBuckets::set(
					now,
					&IncomingTransfers { transfers: admitted, next_slot: None },
				)
				.is_err()
			{
				reject();
			}
			chain.last_slot = now;
			chain.count += added;
			if TransferChain::set(chain).is_err() {
				reject();
			}
		},
	}
	// §5.1: unreserved entries are charged to Asset Hub as they arrive, priced
	// per worst-case bucket rather than by `amount`.
	reattribute_transfer_queue(old_count as u64, queued as u64);
	logs
}

/// §5.1 `consume_transfers_up_to(slot)`: drop whole buckets up to and including
/// `slot`, walking the chain from `first_slot` (Asset Hub only). `slot` is
/// clamped to the candidate's lookup-anchor `anchor`: Asset Hub cannot have read
/// a bucket newer than the anchor it built on, so a slot beyond it must not
/// drain buckets it never observed. Buckets at or below the anchor are still
/// drained normally.
pub fn consume_up_to(slot: Timeslot, anchor: Timeslot) {
	let slot = slot.min(anchor);
	let Some(mut chain) = TransferChain::get() else { return };
	let old_count = chain.count;
	let mut cursor = chain.first_slot;
	loop {
		if cursor > slot {
			chain.first_slot = cursor;
			let _ = TransferChain::set(&chain);
			// §5.1: draining refunds the per-bucket charge of the unreserved
			// entries removed, restoring Asset Hub's allowance.
			reattribute_transfer_queue(old_count as u64, chain.count as u64);
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
				reattribute_transfer_queue(old_count as u64, 0);
				return;
			},
		}
	}
}

/// Replay a `TransferOut` (Asset Hub only) via JAM `transfer` (§5.1 step 7).
///
/// The vendored JAM host is Gray Paper 0.7.2, whose `transfer` always runs the
/// destination's accumulate (the deferred mode), always debits this service, and
/// knows a single balance per service. Three of the spec's shapes therefore have
/// no host support and are refused with the error the design assigns them: a
/// foreign `source`, a plain move (`deferred: None`, which §5.1 also rejects
/// whenever this service does not supervise `dest` — it never does), and either
/// supervisor-balance selector.
/// FIXME: revisit once the host exposes a GP >= 0.8 `transfer`.
pub fn transfer_out(args: TransferOutArgs, logs: &mut Vec<AccumulateLog>) {
	let TransferOutArgs {
		source,
		dest,
		amount,
		id,
		source_supervisor_balance,
		dest_supervisor_balance,
		deferred,
	} = args;
	let mut fail = |error| logs.push(AccumulateLog::TransferFailed { id, error });

	// Only this service's own regular balance is exempt from supervision, so any
	// named source fails; which error depends on whether it exists at all.
	if let Some(source) = source {
		return fail(if service_info(source).is_none() {
			TransferError::UnknownSource
		} else {
			TransferError::SourceNotSupervised
		});
	}
	if source_supervisor_balance || dest_supervisor_balance {
		return fail(TransferError::DestinationNotSupervised);
	}
	let Some((memo, gas)) = deferred else {
		return fail(TransferError::DestinationNotSupervised);
	};
	let Some(info) = service_info(dest) else {
		return fail(TransferError::UnknownDestination);
	};
	if gas < info.min_memo_gas {
		return fail(TransferError::GasBelowDestinationMinimum);
	}
	if gas > MAX_TRANSFER_GAS {
		// `Ω_T` charges the forwarded gas to this service's own accumulate meter,
		// so an unbounded request burns the whole invocation (D-6).
		// FIXME: the design defines no error for a sender-side gas cap.
		return fail(TransferError::InsufficientServiceBalance);
	}
	if transfer(dest, amount.0, gas, &JamMemo(memo)).is_err() {
		fail(TransferError::InsufficientServiceBalance);
	}
}
