//! Per-work-package accumulation (spec §5.1 steps 1–7).

use crate::{
	accumulate::upward,
	hashing::blake2_256,
	head_commitment::HeadTracker,
	state::{
		log::{truncate_auth_trace, AccumulateLog, InsufficientBalanceReason, ParachainLogs},
		para_info::Parachains,
	},
	work_digest::ParachainWorkDigest,
};
use alloc::vec::Vec;
use codec::DecodeAll;
use jam_types::{ServiceId, Slot, WorkItemRecord};

/// Process one work item's result (§5.1). A gray-paper `WorkExecResult::Error`
/// is skipped entirely: no `parachain_log` entry, no state change (§3.3).
pub fn process(now: Slot, service_id: ServiceId, record: &WorkItemRecord, heads: &mut HeadTracker) {
	let Ok(output) = &record.result else { return };
	let digest = ParachainWorkDigest::decode_all(&mut &output[..])
		.expect("refine of this service produced the output; qed");

	match digest {
		ParachainWorkDigest::Err { para_id, error } => {
			// Step 2: a Refine failure is logged with the work-report's
			// authorizer trace (truncated to 256 B) and processing stops.
			// `append_refine` no-ops for an unregistered para (step 1).
			ParachainLogs::append_refine(
				para_id,
				now,
				error,
				truncate_auth_trace(&record.auth_output),
			);
		},
		ParachainWorkDigest::Ok {
			para_id,
			validation_code,
			parent_head_hash,
			head_data,
			upward_messages,
			lookup_anchor,
		} => {
			// Step 1: registration check. A not-registered OR deregistering para
			// is treated as if it no longer exists — no new log entry (§6.4).
			let Some(pi) = Parachains::get(para_id) else { return };
			if pi.is_deregistering {
				return;
			}

			// Step 3: parent-head check — reject candidates built on a stale,
			// skipped, or non-canonical parent.
			if parent_head_hash != blake2_256(&pi.head_data) {
				return;
			}

			// Step 4: authoritative validation-code check. Only the ACTIVE code
			// is accepted; an announced upgrade becomes a validation option only
			// once an `Apply` has made it active (§5.2). Compares the whole
			// `(hash, len)` pair: the preimage registry is keyed by both, so the
			// same hash at another length is another code.
			if pi.validation_code != Some(validation_code) {
				return;
			}

			// §4.3 defense-in-depth: Refine already aborts restricted host
			// functions from the wrong para, but re-verify before applying.
			if upward_messages.iter().any(|m| !m.allowed_for(para_id)) {
				return;
			}

			// Only an accepted candidate may prune logs.
			ParachainLogs::prune_below(para_id, lookup_anchor);
			let mut logs: Vec<AccumulateLog> = Vec::new();

			// Step 5: head-data update.
			let mut pi = Parachains::get(para_id).expect("checked live above; qed");
			heads.touch(para_id);
			pi.head_data = head_data;
			// A head overwrite can grow the `ParaInfo` entry; a backstop write
			// failure (§6.1 invariant) logs the rejection and the
			// rest of the candidate's effects still apply.
			if Parachains::set(para_id, &pi).is_err() {
				logs.push(AccumulateLog::InsufficientStateBalance {
					reason: InsufficientBalanceReason::ParaInfo,
				});
			}

			// Step 6: replay the upward messages in order.
			for message in upward_messages.into_iter() {
				upward::apply(now, service_id, para_id, lookup_anchor, message, &mut logs, heads);
			}

			ParachainLogs::append_accumulate(para_id, now, logs);
		},
	}
}
