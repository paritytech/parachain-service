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
use jam_types::{Memo as JamMemo, TransferRecord};
use parachain_service_interface::{types::BucketId, upward_message::TransferOutArgs};

/// §5.1 incoming-transfer processing. JAM credited the balances before this
/// code runs, so handling is best effort: within the pre-provisioned portion a
/// transfer is recorded unconditionally; beyond it only if its amount covers
/// its own worst-case queue entry. Otherwise it is dropped — no record, no log.
///
/// Admitted transfers are buffered in memory and written once per whole bucket,
/// never transfer by transfer: per-transfer writes would re-read and re-write
/// the growing bucket each time — measured at 55x the `Ga` budget for 1024
/// same-slot transfers (D-8); the resulting state is identical.
///
/// Returns the `InsufficientStateBalance` entries for bucket/queue writes that
/// hit the §6.1 backstop; the caller routes them to Asset Hub's
/// parachain log.
pub fn record_incoming(records: &[&TransferRecord]) -> Vec<AccumulateLog> {
	let mut logs = Vec::new();
	let queue = TransferQueue::get();
	let mut queued = queue.map_or(0, |q| q.count);
	let old_count = queued;

	// §5.1: this invocation opens a fresh bucket rather than appending to the
	// one the previous invocation left, and rolls over to the next id whenever
	// the open bucket reaches `MAX_TRANSFERS_PER_BUCKET`. Buckets are filled in
	// memory and written once each (D-8), so a full digest costs one write per
	// bucket rather than one per transfer.
	let mut next_id = queue.map_or(0, |q| q.last_bucket + 1);
	let mut filled: Vec<(BucketId, IncomingTransfers)> = Vec::new();
	for record in records {
		// §5.1: inside the reservation the entry is already paid for.
		if (queued as usize) >= MAX_INCOMING_TRANSFERS && !transfer_covers_own_slot(record.amount) {
			continue;
		}
		queued += 1;
		let transfer =
			QueuedTransfer { from: record.source, amount: record.amount, memo: record.memo.0 };
		match filled.last_mut() {
			Some((_, open)) if open.len() < MAX_TRANSFERS_PER_BUCKET as usize => {
				open.try_push(transfer).expect("checked the bound above; qed");
			},
			_ => {
				let mut open = IncomingTransfers::new();
				open.try_push(transfer).expect("a fresh bucket has room; qed");
				filled.push((next_id, open));
				next_id += 1;
			},
		}
	}
	let Some((first_new, _)) = filled.first() else { return logs };
	let last_new = next_id - 1;
	let mut reject = || {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::IncomingTransfer,
		});
	};

	// The endpoints are advanced only once every bucket landed, so a backstop
	// failure never leaves them naming a bucket that was not written.
	for (id, bucket) in &filled {
		if TransferBuckets::set(*id, bucket).is_err() {
			reject();
			return logs;
		}
	}
	let endpoints = IncomingTransferBuckets {
		first_bucket: queue.map_or(*first_new, |q| q.first_bucket),
		last_bucket: last_new,
		count: queued,
	};
	if TransferQueue::set(&endpoints).is_err() {
		reject();
		return logs;
	}
	// §5.1: unreserved entries are charged to Asset Hub as they arrive, priced
	// per worst-case bucket rather than by `amount`.
	reattribute_transfer_queue(old_count as u64, queued as u64);
	logs
}

/// §5.1 `clean_up_buckets_up_to(bucket_id)`: remove whole buckets from
/// `first_bucket` up to and including `bucket_id`, pointing `first_bucket` at the
/// first survivor (Asset Hub only). Once nothing remains the endpoint entry is
/// removed, so ids restart from `0` rather than increasing forever.
///
/// No clamping is needed: as long as the JAM block the parachain references only
/// ever advances, it can only name buckets it has actually seen, so it can never
/// remove one it has not read.
pub fn clean_up_buckets_up_to(bucket_id: BucketId) {
	let Some(mut queue) = TransferQueue::get() else { return };
	// Already-removed ids name nothing, and must not drag `first_bucket` back.
	if bucket_id < queue.first_bucket {
		return;
	}
	let old_count = queue.count;
	// Ids are contiguous, so the survivors are just the tail of the range.
	let last_removed = bucket_id.min(queue.last_bucket);
	for id in queue.first_bucket..=last_removed {
		let removed = TransferBuckets::get(id).map_or(0, |b| b.len() as u32);
		queue.count = queue.count.saturating_sub(removed);
		TransferBuckets::remove(id);
	}
	if last_removed >= queue.last_bucket {
		TransferQueue::clear();
		reattribute_transfer_queue(old_count as u64, 0);
		return;
	}
	queue.first_bucket = last_removed + 1;
	let _ = TransferQueue::set(&queue);
	// §5.1: clean-up refunds the per-bucket charge of the unreserved entries
	// removed, restoring Asset Hub's allowance.
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
