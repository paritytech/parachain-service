//! Incoming-transfer processing and outbound-transfer replay (spec §5.1).

use crate::{
	constants::{MAX_INCOMING_TRANSFERS, MAX_TRANSFERS_PER_BUCKET, MAX_TRANSFER_GAS},
	state::{
		log::{AccumulateLog, InsufficientBalanceReason, TransferError},
		transfers::{
			IncomingTransferBuckets, IncomingTransfers, QueuedTransfer, TransferBuckets,
			TransferQueue,
		},
	},
	state_balance::{reattribute_transfer_queue, transfer_covers_own_slot},
};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::{service_info, transfer};
use jam_types::{Memo as JamMemo, Slot, TransferRecord};
use parachain_service_interface::upward_message::{BucketId, TransferOutArgs};

/// §5.1 incoming-transfer processing. JAM credited the balances before this
/// code runs, so handling is best effort: within the pre-provisioned portion a
/// transfer is recorded unconditionally; beyond it only if its amount covers
/// its own worst-case queue entry. Otherwise it is dropped — no record, no log.
///
/// Each invocation opens fresh contiguous buckets, packing at most
/// `MAX_TRANSFERS_PER_BUCKET` admitted transfers into each one.
///
/// Returns the `InsufficientStateBalance` entries for bucket/endpoint writes that
/// hit the §6.1 backstop; the caller routes them to Asset Hub's
/// parachain log.
pub fn record_incoming(_now: Slot, records: &[&TransferRecord]) -> Vec<AccumulateLog> {
	let mut logs = Vec::new();
	let mut queue = TransferQueue::get();
	let mut queued = queue.as_ref().map_or(0, |q| q.count);
	let old_count = queued;
	let mut admitted = Vec::new();
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
	let mut reject = || {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::IncomingTransfer,
		});
	};

	let first_new = queue.as_ref().map_or(0, |q| q.last_bucket.saturating_add(1));
	let mut last_new = first_new;
	for (offset, chunk) in admitted.chunks(MAX_TRANSFERS_PER_BUCKET).enumerate() {
		let id = first_new.saturating_add(offset as BucketId);
		last_new = id;
		if TransferBuckets::set(id, &IncomingTransfers { transfers: chunk.to_vec() }).is_err() {
			reject();
		}
	}
	let first_bucket = queue.as_ref().map_or(first_new, |q| q.first_bucket);
	queue = Some(IncomingTransferBuckets { first_bucket, last_bucket: last_new, count: queued });
	if TransferQueue::set(queue.as_ref().expect("just set")).is_err() {
		reject();
	}
	// §5.1: unreserved entries are charged to Asset Hub as they arrive, priced
	// per worst-case bucket rather than by `amount`.
	reattribute_transfer_queue(old_count as u64, queued as u64);
	logs
}

/// §5.1 `clean_up_buckets_up_to(id)`: remove whole contiguous buckets up to and
/// including `id` (Asset Hub only).
pub fn clean_up_buckets_up_to(id: BucketId) {
	let Some(mut queue) = TransferQueue::get() else { return };
	let old_count = queue.count;
	let last_removed = id.min(queue.last_bucket);
	if last_removed < queue.first_bucket {
		return;
	}
	for bucket_id in queue.first_bucket..=last_removed {
		let bucket = TransferBuckets::get(bucket_id).expect("queue ids are contiguous; qed");
		queue.count = queue.count.saturating_sub(bucket.transfers.len() as u32);
		TransferBuckets::remove(bucket_id);
	}
	if last_removed == queue.last_bucket {
		TransferQueue::clear();
	} else {
		queue.first_bucket = last_removed + 1;
		let _ = TransferQueue::set(&queue);
	}
	reattribute_transfer_queue(old_count as u64, queue.count as u64);
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
