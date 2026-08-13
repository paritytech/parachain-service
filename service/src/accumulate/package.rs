//! Per-work-package accumulation (spec §5.1 steps 1–7).

use crate::{
	accumulate::{code_upgrades, upward},
	hashing::blake2_256,
	state::{
		log::{truncate_auth_trace, AccumulateLog, ParachainLogs},
		para_info::Parachains,
	},
	work_digest::ParachainWorkDigest,
};
use alloc::vec::Vec;
use codec::DecodeAll;
use jam_types::{ServiceId, Slot, WorkItemRecord};

/// Process one work item's result (§5.1). A gray-paper `WorkExecResult::Error`
/// is skipped entirely: no `parachain_log` entry, no state change (§3.3).
pub fn process(now: Slot, service_id: ServiceId, record: &WorkItemRecord) {
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

			let mut logs: Vec<AccumulateLog> = Vec::new();

			'candidate: {
				// Step 3: parent-head check — reject candidates built on a
				// stale, skipped, or non-canonical parent. Silent (no log).
				if parent_head_hash != blake2_256(&pi.head_data) {
					break 'candidate;
				}

				// Step 4: lazily reap a timed-out pending upgrade.
				code_upgrades::reap_timed_out_upgrade(para_id, now, &mut logs);

				// Step 5: authoritative validation-code check, against the
				// (possibly just-reaped) ParaInfo.
				let pi = Parachains::get(para_id).expect("checked live above; qed");
				let matches_active =
					pi.validation_code.as_ref().is_some_and(|vc| vc.code_ref == validation_code);
				let matches_pending = pi
					.pending_upgrade
					.as_ref()
					.is_some_and(|(vc, _)| vc.code_ref == validation_code);
				if !matches_active && !matches_pending {
					logs.push(AccumulateLog::InvalidCodeHash { hash: validation_code.hash });
					break 'candidate;
				}

				// §4.3 defense-in-depth: Refine already aborts restricted host
				// functions from the wrong para, but re-verify before applying.
				// TODO: the Quint model logs `InvalidCodeHash` here, which is
				// misleading; we reject silently. Needs upstreaming.
				if upward_messages.iter().any(|m| !m.allowed_for(para_id)) {
					break 'candidate;
				}

				// Step 6: head-data update + code-upgrade activation. Activation
				// must happen before the replay so a new upgrade request in the
				// same digest arms against the just-activated code (§5.2).
				let mut pi = pi;
				pi.head_data = head_data;
				Parachains::set(para_id, &pi);
				code_upgrades::activate_upgrade_if_match(
					para_id,
					validation_code.hash,
					now,
					&mut logs,
				);

				// Step 7: replay the upward messages in order.
				for message in upward_messages.into_iter() {
					upward::apply(now, service_id, para_id, message, &mut logs);
				}
			}

			// §5.1 log pruning: entries older than the candidate's lookup-anchor
			// are dropped before this candidate's events are appended.
			ParachainLogs::prune_below(para_id, lookup_anchor);
			ParachainLogs::append_accumulate(para_id, now, logs);
		},
	}
}
