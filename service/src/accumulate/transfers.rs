//! Incoming-transfer processing and outbound-transfer replay (spec §5.1).

use crate::{
	constants::{MAX_INCOMING_TRANSFERS, MAX_TRANSFERS_PER_BUCKET},
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
use parachain_service_core::{
	types::{BucketId, ServiceId},
	upward_message::TransferOutArgs,
};

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
		let transfer = QueuedTransfer {
			from: record.source,
			amount: record.amount,
			// FIXME: the vendored JAM TransferRecord has no destination-balance selector.
			// Its transfers only credit the regular balance; preserve the real flag once
			// the host supports supervisor balances (DIVERGENCE.md M-12).
			to_supervisor_balance: false,
			memo: record.memo.0,
		};
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

/// Replay a `TransferOut` (Asset Hub only) via JAM `transfer` (§5.1 step 6),
/// refusing in the order JAM checks: source, destination, control of the source,
/// a plain move's target, the destination's gas minimum, the source's funds.
///
/// The vendored host's `transfer` always runs the destination's accumulate (the
/// deferred mode), always debits this service, and knows one balance per service.
/// So this service controls only itself, and its supervisor balance is always
/// empty. What needs more is refused: a plain move to anything but its own other
/// balance, and a non-zero credit to a supervisor balance (D-11).
/// FIXME: revisit once the host exposes a GP >= 0.8 `transfer`.
pub fn transfer_out(service_id: ServiceId, args: TransferOutArgs, logs: &mut Vec<AccumulateLog>) {
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

	let foreign_source = source.filter(|&source| source != service_id);
	if foreign_source.is_some_and(|source| service_info(source).is_none()) {
		return fail(TransferError::UnknownSource);
	}
	let Some(dest_info) = service_info(dest) else {
		return fail(TransferError::UnknownDestination);
	};
	if foreign_source.is_some() {
		return fail(TransferError::SourceNotSupervised);
	}
	let moves = amount.0 > 0;
	let Some((memo, gas)) = deferred else {
		if dest != service_id || source_supervisor_balance == dest_supervisor_balance {
			return fail(TransferError::DestinationNotSupervised);
		}
		if moves {
			fail(if source_supervisor_balance {
				TransferError::InsufficientServiceBalance
			} else {
				TransferError::DestinationNotSupervised
			});
		}
		return;
	};
	if gas < dest_info.min_memo_gas {
		return fail(TransferError::GasBelowDestinationMinimum);
	}
	if moves && source_supervisor_balance {
		return fail(TransferError::InsufficientServiceBalance);
	}
	if moves && dest_supervisor_balance {
		return fail(TransferError::DestinationNotSupervised);
	}
	if transfer(dest, amount.0, gas, &JamMemo(memo)).is_err() {
		fail(TransferError::InsufficientServiceBalance);
	}
}
