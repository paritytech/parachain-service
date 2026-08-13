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
use parachain_service_interface::types::{Balance, Hash, ParaId, Timeslot, ValidationCodeHash};

/// Why a state-balance reservation failed (spec §3.1, §6.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum InsufficientBalanceReason {
	/// A `solicit` (or code-upgrade solicit) of the preimage with `hash` and `len`.
	Solicit { hash: Hash, len: Compact<u32> },
	/// A `kv_set(key, value)` write. Only the hash of `key` is recorded so an
	/// arbitrarily large user key cannot inflate `parachain_log`.
	SetKV { key_hash: Hash },
}

/// Events recorded while Accumulating for a parachain (spec §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum AccumulateLog {
	/// The work digest's `validation_code_hash` matches neither the active code
	/// nor the pending upgrade. Spec §5.1 step 5.
	InvalidCodeHash { hash: ValidationCodeHash },
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
	/// The JAM `transfer` replaying a `TransferOut` failed. Spec §5.1 step 7.
	TransferFailed { memo_hash: Hash },
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

/// Append `entry`, then evict until the log's exact encoded size fits the 64 KiB
/// cap: the oldest `Refine` entry goes first (failures are the most disposable);
/// only once no refine entry remains is the oldest entry overall dropped (§5.1).
pub fn push_log_entry(log: &mut ParachainLog, entry: (Timeslot, LogEntry)) {
	log.push(entry);
	while log.encoded_size() > PARACHAIN_LOG_BYTE_CAP && !log.is_empty() {
		let victim =
			log.iter().position(|(_, e)| matches!(e, LogEntry::Refine { .. })).unwrap_or(0);
		log.remove(victim);
	}
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
