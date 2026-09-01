//! Per-work-package accumulation (spec §5.1 steps 1–7).

use crate::{
	accumulate::{code_upgrades, upward},
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
			// is treated as if it no longer exists — silent drop, no log (§6.4).
			let Some(pi) = Parachains::get(para_id) else { return };
			if pi.is_deregistering {
				return;
			}

			// Steps 3-5 only decide accept/reject. A candidate rejected at any of
			// them changes nothing at all: no state, no log entry, no pruning
			// (§5.1) — so nothing below writes until every check has passed.

			// Step 3: parent-head check — reject candidates built on a stale,
			// skipped, or non-canonical parent.
			if parent_head_hash != blake2_256(&pi.head_data) {
				return;
			}

			// Step 4 (decide): a pending upgrade past its deadline is expired
			// before this candidate is considered, so step 5 must not match
			// against it. The release itself is deferred to the accept path.
			let expired = code_upgrades::pending_upgrade_expired(&pi, now);

			// Step 5: authoritative validation-code check, on the post-reap view.
			// Compares the whole `(hash, len)` pair: the preimage registry is
			// keyed by both, so the same hash at another length is another code.
			let matches_active =
				pi.validation_code.as_ref().is_some_and(|vc| vc.code_ref == validation_code);
			let matches_pending = !expired &&
				pi.pending_upgrade
					.as_ref()
					.is_some_and(|(vc, _)| vc.code_ref == validation_code);
			if !matches_active && !matches_pending {
				return;
			}

			// §4.3 defense-in-depth: Refine already aborts restricted host
			// functions from the wrong para, but re-verify before applying.
			if upward_messages.iter().any(|m| !m.allowed_for(para_id)) {
				return;
			}

			// The candidate is accepted; its effects may now be applied.
			let mut logs: Vec<AccumulateLog> = Vec::new();

			// §5.1: an accepted candidate prunes entries below its lookup-anchor
			// before any of its own effects land. Only accepted candidates prune —
			// the anchor is chosen by whoever submitted the package, so letting a
			// rejected one prune would let anyone holding coretime wipe the log.
			ParachainLogs::prune_below(para_id, lookup_anchor);

			// Step 4 (apply): release the expired pending code.
			if expired {
				code_upgrades::reap_timed_out_upgrade(para_id, now, &mut logs);
			}

			// Step 6: head-data update + code-upgrade activation. Activation must
			// happen before the replay so a new upgrade request in the same digest
			// arms against the just-activated code (§5.2).
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
			code_upgrades::activate_upgrade_if_match(para_id, validation_code.hash, now, &mut logs);

			// Step 7: replay the upward messages in order.
			for message in upward_messages.into_iter() {
				upward::apply(now, service_id, para_id, message, &mut logs, heads);
			}

			ParachainLogs::append_accumulate(para_id, now, logs);
		},
	}
}
