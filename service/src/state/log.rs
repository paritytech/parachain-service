//! Per-parachain log (spec §3.1, §5.1): entry types, exact-size accounting,
//! eviction, and pruning.
//!
//! Unlike the Quint model's approximated `logEntrySize`, sizes here are the
//! exact SCALE `encoded_size()` of the stored value, so the 64 KiB cap is
//! enforced against real bytes (this is what SPEC_GAPS #17 asks for).

use crate::{
	constants::{PARACHAIN_LOG_BYTE_CAP, STORED_AUTH_TRACE_CAP},
	state::{self, Tag},
	work_digest::RefineLog,
};
use alloc::vec::Vec;
use bounded_collections::{BoundedVec, ConstU32};
use codec::{Compact, Decode, Encode};
use parachain_service_interface::types::{Balance, Hash, ParaId, Timeslot};

/// Why a state-balance reservation failed (spec §3.1, §6.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum InsufficientBalanceReason {
	/// A `solicit` (or code-upgrade solicit) of the preimage with `hash` and `len`.
	Solicit { hash: Hash, len: Compact<u32> },
	/// A `kv_set(key, value)` write. Only the hash of `key` is recorded so an
	/// arbitrarily large user key cannot inflate `parachain_log`.
	SetKV { key_hash: Hash },
}

/// Why a JAM `transfer` replaying a `TransferOut` failed (spec §5.1 step 7).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum TransferError {
	/// `source` is not a known service.
	UnknownSource,
	/// `dest` is not a known service.
	UnknownDestination,
	/// The service is not `source`'s effective supervisor; only its own regular
	/// balance is exempt. Takes precedence over `DestinationNotSupervised`.
	SourceNotSupervised,
	/// A plain move to another service needs the service to be `dest`'s effective
	/// supervisor. Also covers an identity write (`source == dest` with both
	/// selectors equal).
	DestinationNotSupervised,
	/// The supplied gas is below `dest`'s `min_memo_gas`.
	GasBelowDestinationMinimum,
	/// The transfer would leave the Parachain Service below its threshold balance.
	InsufficientServiceBalance,
}

/// Events recorded while Accumulating for a parachain (spec §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum AccumulateLog {
	/// Available state balance insufficient for the operation. Spec §6.1.
	InsufficientStateBalance { reason: InsufficientBalanceReason },
	/// `parachain_set_state_balance` rejected because `attempted < current_used`.
	StateBalanceUpdateRejected {
		attempted: Compact<Balance>,
		current_total: Compact<Balance>,
		current_used: Compact<Balance>,
	},
	/// JAM `designate` was not called: the assembled key set's length is not in
	/// `valcount`. The staging buffer is cleared regardless. Spec §5.3.
	DesignateRejected { len: Compact<u32> },
	/// A `set_validator_keys` chunk would overflow `staged_validator_keys`;
	/// the append is rejected. Spec §5.3.
	StagedValidatorKeysOverflow,
	/// `parachain_service_upgrade` rejected: new code's preimage missing. Spec §5.4.
	ServiceUpgradePreimageMissing { code_hash: Hash },
	/// The JAM `transfer` replaying a `TransferOut` failed. `id` is the
	/// caller-supplied identifier, echoed back so the parachain can match the
	/// failure to its own record. Spec §5.1 step 7.
	TransferFailed { id: Compact<u64>, error: TransferError },
	/// A `forget` removed the last referencer without expunging the preimage;
	/// `forget` again once strictly past `due`. Spec §6.1.
	ForgetAgainAt { hash: Hash, len: Compact<u32>, due: Timeslot },
	/// `parachain_clean_up` rejected: state held beyond baseline + codes. Spec §6.4.
	TooMuchStateHeld,
}

/// The auth trace stored with a Refine failure, truncated to 256 bytes (§3.3).
pub type StoredAuthTrace = BoundedVec<u8, ConstU32<{ STORED_AUTH_TRACE_CAP as u32 }>>;

/// One log entry (spec §3.1). Each carries its timeslot inline in the log vector.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum LogEntry {
	/// What went wrong during Refine, with the work-report's auth trace.
	Refine { error: RefineLog, auth_trace: StoredAuthTrace },
	/// Events recorded while Accumulating a work digest.
	Accumulate { entries: Vec<AccumulateLog> },
}

/// A parachain's full log value: `(timeslot, entry)` pairs in append order.
pub type ParachainLog = Vec<(Timeslot, LogEntry)>;

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
		state::read(Tag::ParachainLog, &para_id).unwrap_or_default()
	}

	pub fn set(para_id: ParaId, log: &ParachainLog) {
		state::write(Tag::ParachainLog, &para_id, log)
	}

	pub fn remove(para_id: ParaId) {
		state::clear(Tag::ParachainLog, &para_id)
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
		Self::set(para_id, &log);
	}

	/// Append one batched `Accumulate` entry for a live para (§5.1). No-op for an
	/// unregistered para or an empty batch.
	pub fn append_accumulate(para_id: ParaId, now: Timeslot, entries: Vec<AccumulateLog>) {
		if entries.is_empty() || !super::para_info::Parachains::is_live(para_id) {
			return;
		}
		let mut log = Self::get(para_id);
		push_log_entry(&mut log, (now, LogEntry::Accumulate { entries }));
		Self::set(para_id, &log);
	}

	/// §5.1 pruning against the candidate's lookup-anchor. No-op without a log.
	pub fn prune_below(para_id: ParaId, threshold: Timeslot) {
		let mut log = Self::get(para_id);
		if log.is_empty() {
			return;
		}
		prune_log_below(&mut log, threshold);
		Self::set(para_id, &log);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn refine_entry(trace_len: usize) -> LogEntry {
		LogEntry::Refine {
			error: RefineLog::InvalidCodeHash,
			auth_trace: truncate_auth_trace(&alloc::vec![0xAB; trace_len]),
		}
	}

	fn accumulate_entry() -> LogEntry {
		LogEntry::Accumulate { entries: alloc::vec![AccumulateLog::TooMuchStateHeld] }
	}

	fn big_accumulate_entry(events: usize) -> LogEntry {
		LogEntry::Accumulate { entries: alloc::vec![AccumulateLog::TooMuchStateHeld; events] }
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
