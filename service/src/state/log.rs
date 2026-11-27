//! Per-parachain log (spec §3.1, §5.1): eviction, pruning and storage accessors.
//!
//! The entry types and their sizing constants live in `parachain-service-core::log`,
//! because a parachain's own validation code decodes them from its JAM state proof —
//! the feedback channel of §5.1 — and both sides must agree on every discriminant.
//! Unlike the Quint model's approximated `logEntrySize`, sizes here are the
//! exact SCALE `encoded_size()` of the stored value, so the 64 KiB cap is
//! enforced against real bytes.

use crate::{
	constants::{PARACHAIN_LOG_BYTE_CAP, STORED_AUTH_TRACE_CAP},
	state::{self, StorageFull, Tag},
};
use alloc::vec::Vec;
use codec::Encode;
use parachain_service_core::types::{MessageId, ParaId, Timeslot};

pub use parachain_service_core::log::{
	AccumulateLog, LogEntry, ParachainLog, RefineLog, ServiceCreationResult, ServiceEjectError,
	ServiceSolicitError, ServiceSupervisorError, StateBalanceRejection, StoredAuthTrace,
	TransferError,
};

/// Truncate a work-report auth trace to the stored cap (§3.3).
pub fn truncate_auth_trace(trace: &[u8]) -> StoredAuthTrace {
	let end = trace.len().min(STORED_AUTH_TRACE_CAP);
	trace[..end].to_vec().try_into().expect("truncated to the bound; qed")
}

/// Eviction rank of a log entry (§5.1). Lower ranks are discarded first:
///
/// - 0 — a refine error other than `Opaque`: a fixed structural failure carrying no
///   parachain-supplied detail, and the rank every failure a coretime buyer can provoke falls into.
///   The most disposable of the three.
/// - 1 — `Opaque`: the payload the parachain's own PVF chose to report (§4.2), the only refine
///   entry carrying context the parachain can act on.
/// - 2 — an accumulate event: an actual on-chain state change.
pub fn entry_rank(entry: &(Timeslot, LogEntry)) -> u8 {
	match &entry.1 {
		LogEntry::Refine { error: RefineLog::Opaque(_), .. } => 1,
		LogEntry::Refine { .. } => 0,
		LogEntry::Accumulate { .. } => 2,
	}
}

/// Drop the single best eviction candidate on behalf of an incoming entry of rank
/// `rank`: the oldest entry of the lowest occupied rank at or below `rank`.
/// Nothing above `rank` is ever touched, so a low-ranked newcomer can never
/// displace a higher-ranked entry. `false` when every entry present outranks it.
fn evict_one_for(log: &mut ParachainLog, rank: u8) -> bool {
	// Age is read from the stored timeslot rather than list position, so eviction
	// does not depend on the log being kept in order.
	let victim = log
		.iter()
		.enumerate()
		.filter(|(_, e)| entry_rank(e) <= rank)
		.min_by_key(|(_, e)| (entry_rank(e), e.0))
		.map(|(i, _)| i);
	match victim {
		Some(i) => {
			log.remove(i);
			true
		},
		None => false,
	}
}

/// Append `entry`, then evict by rank until the log's exact encoded size fits the
/// 64 KiB cap (§5.1).
///
/// The newcomer is itself a candidate at its own rank and carries the highest
/// timeslot, so it is picked only once every other entry at or below its rank is
/// gone — which is exactly the spec's "the incoming entry is dropped instead"
/// rule for a newcomer that cannot displace what remains.
pub fn push_log_entry(log: &mut ParachainLog, entry: (Timeslot, LogEntry)) {
	let rank = entry_rank(&entry);
	log.push(entry);
	while log.encoded_size() > PARACHAIN_LOG_BYTE_CAP && evict_one_for(log, rank) {}
}

/// §5.1 log pruning: drop entries whose inline timeslot is strictly below the
/// candidate's lookup-anchor `threshold`.
pub fn prune_log_below(log: &mut ParachainLog, threshold: Timeslot) {
	log.retain(|(slot, _)| *slot >= threshold);
}

/// Storage accessors for the `parachain_log` map (tag `0x01`).
pub struct ParachainLogs;

impl ParachainLogs {
	pub fn get(para_id: ParaId) -> ParachainLog {
		state::read(&Tag::ParachainLog.para_key(para_id)).unwrap_or_default()
	}

	/// Persist the log. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(para_id: ParaId, log: &ParachainLog) -> Result<(), StorageFull> {
		state::write(&Tag::ParachainLog.para_key(para_id), log)
	}

	pub fn remove(para_id: ParaId) {
		state::clear(&Tag::ParachainLog.para_key(para_id))
	}

	/// Append a Refine failure for a live para (§3.3). No-op for an unregistered
	/// para — there is no log to append to (§5.1 step 1).
	pub fn append_refine(
		para_id: ParaId,
		now: Timeslot,
		error: RefineLog,
		auth_trace: StoredAuthTrace,
	) {
		if !super::para_info::Parachains::is_live(para_id) {
			return;
		}
		let mut log = Self::get(para_id);
		push_log_entry(&mut log, (now, LogEntry::Refine { error, auth_trace }));
		// A failed write drops the entry: the log is the only channel for its own
		// failure. Best effort; no panic (§6.1 backstop).
		let _ = Self::set(para_id, &log);
	}

	/// Append one batched `Accumulate` entry for a live para (§5.1). No-op for an
	/// unregistered para or an empty batch. Each event keeps the `message_id` of
	/// the upward message that caused it.
	pub fn append_accumulate(
		para_id: ParaId,
		now: Timeslot,
		entries: Vec<(MessageId, AccumulateLog)>,
	) {
		if entries.is_empty() || !super::para_info::Parachains::is_live(para_id) {
			return;
		}
		let mut log = Self::get(para_id);
		push_log_entry(&mut log, (now, LogEntry::Accumulate { entries }));
		// See `append_refine`: a failed write drops the batch, no panic.
		let _ = Self::set(para_id, &log);
	}

	/// §5.1 pruning against the candidate's lookup-anchor. No-op without a log.
	pub fn prune_below(para_id: ParaId, threshold: Timeslot) {
		let mut log = Self::get(para_id);
		if log.is_empty() {
			return;
		}
		prune_log_below(&mut log, threshold);
		// See `append_refine`: a failed write is dropped, no panic.
		let _ = Self::set(para_id, &log);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn refine_entry(trace_len: usize) -> LogEntry {
		LogEntry::Refine {
			error: RefineLog::ValidationCodeLookupFailed,
			auth_trace: truncate_auth_trace(&alloc::vec![0xAB; trace_len]),
		}
	}

	fn accumulate_entry() -> LogEntry {
		LogEntry::Accumulate {
			entries: alloc::vec![(0u64.into(), AccumulateLog::TooMuchStateHeld)],
		}
	}

	fn big_accumulate_entry(events: usize) -> LogEntry {
		LogEntry::Accumulate {
			entries: alloc::vec![(0u64.into(), AccumulateLog::TooMuchStateHeld); events],
		}
	}

	fn opaque_entry(payload_len: usize) -> LogEntry {
		LogEntry::Refine {
			error: RefineLog::Opaque(
				alloc::vec![0xCD; payload_len].try_into().expect("within the 1024 bound; qed"),
			),
			auth_trace: truncate_auth_trace(&[]),
		}
	}

	fn count_rank(log: &ParachainLog, rank: u8) -> usize {
		log.iter().filter(|e| entry_rank(e) == rank).count()
	}

	#[test]
	fn entry_rank_order_works() {
		assert_eq!(entry_rank(&(0, refine_entry(4))), 0);
		assert_eq!(entry_rank(&(0, opaque_entry(8))), 1);
		assert_eq!(entry_rank(&(0, accumulate_entry())), 2);
	}

	#[test]
	fn spam_cannot_evict_opaque_works() {
		// The property the ranking exists for (§5.1): anyone can buy coretime and
		// provoke rank-0 failures against a parachain, but they must never displace
		// the parachain's own `Opaque` reports.
		let mut log = ParachainLog::new();
		let mut slot = 0;
		while log.encoded_size() < PARACHAIN_LOG_BYTE_CAP - 2_000 {
			slot += 1;
			push_log_entry(&mut log, (slot, opaque_entry(1014)));
		}
		let opaques_before = count_rank(&log, 1);
		assert!(opaques_before > 0);

		for _ in 0..40 {
			slot += 1;
			push_log_entry(&mut log, (slot, refine_entry(256)));
		}

		assert_eq!(count_rank(&log, 1), opaques_before);
		assert!(log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP);
	}

	#[test]
	fn higher_rank_evicts_lower_works() {
		// The converse (§5.1): the ranking is not "reject everything once full" —
		// an accumulate entry is admitted at the cost of refine entries.
		let mut log = ParachainLog::new();
		let mut slot = 0;
		while log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP - 300 {
			slot += 1;
			push_log_entry(&mut log, (slot, refine_entry(256)));
		}
		let spam_before = count_rank(&log, 0);
		assert!(spam_before > 0);

		push_log_entry(&mut log, (slot + 1, big_accumulate_entry(500)));

		assert_eq!(count_rank(&log, 2), 1);
		assert!(count_rank(&log, 0) < spam_before);
		assert!(log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP);
	}

	#[test]
	fn incoming_dropped_when_outranked_works() {
		// §5.1: an entry is never evicted to make room for something of lower rank
		// — when only higher-ranked entries remain the newcomer is dropped instead.
		// A rank-0 newcomer arriving at a log holding only higher ranks must leave
		// it byte-for-byte untouched. The log is packed with `Opaque` and then
		// padded with tiny accumulate entries until under 8 bytes of headroom
		// remain, so the newcomer cannot simply fit alongside them.
		let mut log = ParachainLog::new();
		let mut slot = 0;
		while log.encoded_size() < PARACHAIN_LOG_BYTE_CAP - 2_000 {
			slot += 1;
			push_log_entry(&mut log, (slot, opaque_entry(1014)));
		}
		while log.encoded_size() + 8 <= PARACHAIN_LOG_BYTE_CAP {
			slot += 1;
			push_log_entry(&mut log, (slot, accumulate_entry()));
		}
		let before = log.clone();
		assert!(count_rank(&before, 1) > 0);

		push_log_entry(&mut log, (slot + 1, refine_entry(256)));

		assert_eq!(log, before);
		assert_eq!(count_rank(&log, 0), 0);
		assert!(log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP);
	}

	#[test]
	fn trivial_works() {
		let mut log = ParachainLog::new();
		push_log_entry(&mut log, (5, refine_entry(10)));
		assert_eq!(log.len(), 1);
	}

	#[test]
	fn eviction_prefers_refine_entries_works() {
		let mut log = ParachainLog::new();
		// Fill close to the cap with alternating entries.
		while log.encoded_size() < PARACHAIN_LOG_BYTE_CAP - 600 {
			push_log_entry(&mut log, (1, refine_entry(256)));
			push_log_entry(&mut log, (1, accumulate_entry()));
		}
		let accumulates_before =
			log.iter().filter(|(_, e)| matches!(e, LogEntry::Accumulate { .. })).count();
		// Overflow the cap: refine entries must be evicted, accumulate ones kept.
		push_log_entry(&mut log, (2, refine_entry(256)));
		push_log_entry(&mut log, (2, refine_entry(256)));
		let accumulates_after =
			log.iter().filter(|(_, e)| matches!(e, LogEntry::Accumulate { .. })).count();
		assert_eq!(accumulates_before, accumulates_after);
		assert!(log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP);
	}

	#[test]
	fn eviction_drops_oldest_overall_when_no_refine_works() {
		// Build an over-cap log by hand (bypassing eviction), then trigger one
		// evicting push.
		let mut log = ParachainLog::new();
		while log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP {
			let slot = log.len() as Timeslot;
			log.push((slot, accumulate_entry()));
		}
		push_log_entry(&mut log, (u32::MAX, accumulate_entry()));

		assert!(log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP);
		// The oldest entries (lowest slots) were dropped, the new one survives.
		assert_ne!(log[0].0, 0);
		assert_eq!(log.last().expect("non-empty; qed").0, u32::MAX);
	}

	#[test]
	fn prune_below_works() {
		let mut log = ParachainLog::new();
		push_log_entry(&mut log, (1, refine_entry(1)));
		push_log_entry(&mut log, (5, refine_entry(1)));
		push_log_entry(&mut log, (9, refine_entry(1)));
		prune_log_below(&mut log, 5);
		assert_eq!(log.iter().map(|(t, _)| *t).collect::<Vec<_>>(), alloc::vec![5, 9]);
	}

	#[test]
	fn truncate_auth_trace_works() {
		assert_eq!(truncate_auth_trace(&[1u8; 300]).len(), STORED_AUTH_TRACE_CAP);
		assert_eq!(truncate_auth_trace(&[1u8; 3]).len(), 3);
	}
}
