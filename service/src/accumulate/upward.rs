//! Replay of the upward messages carried in a work digest (spec §5.1 step 7).
//!
//! Each variant corresponds 1:1 to a §4.3 side-effect host function. Restriction
//! enforcement happened in Refine (D-2) and is re-checked package-wide before the
//! replay starts (see `package.rs`).

use crate::{
	accumulate::{assigns, code_upgrades, management, transfers, validator_keys},
	state::{log::AccumulateLog, para_info::Parachains},
	state_balance,
};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::{is_available, upgrade};
use jam_types::{CodeHash, ServiceId, Slot};
use parachain_service_interface::{types::ParaId, upward_message::UpwardMessage};

/// Apply one upward message emitted by `origin`'s PVF. Log entries are batched
/// into `logs` and appended to the origin's `parachain_log` by the caller.
pub fn apply(
	now: Slot,
	service_id: ServiceId,
	origin: ParaId,
	message: UpwardMessage,
	logs: &mut Vec<AccumulateLog>,
) {
	match message {
		UpwardMessage::RequestCodeUpgrade { hash, len } => {
			code_upgrades::request_code_upgrade(origin, now, hash, len.0, logs)
		},

		UpwardMessage::Solicit { hash, len } => {
			// For the para's own active/pending validation code this only sets
			// `pinned`: the code is already referenced by the service, so no
			// extra balance is charged (§5.2).
			let mut pi = Parachains::get(origin).expect("origin is live per step 1; qed");
			if let Some(vc) = &mut pi.validation_code {
				if vc.code_ref.is(&hash, len.0) {
					vc.pinned = true;
					Parachains::set(origin, &pi);
					return;
				}
			}
			if let Some((vc, _)) = &mut pi.pending_upgrade {
				if vc.code_ref.is(&hash, len.0) {
					vc.pinned = true;
					Parachains::set(origin, &pi);
					return;
				}
			}
			if let Err(log) = state_balance::add_referencer(origin, &hash, len.0) {
				logs.push(log);
			}
		},

		UpwardMessage::Forget { para_id, hash, len } => {
			// `para_id` names whose reference is released (Coretime may name any
			// para, §6.4); a dead target is a no-op.
			let Some(mut pi) = Parachains::get(para_id) else { return };
			// Forgetting the target's own active/pending code only clears
			// `pinned`; the service still needs the code available, so the
			// referencer stays and no JAM `forget` is forwarded (§5.2).
			if let Some(vc) = &mut pi.validation_code {
				if vc.code_ref.is(&hash, len.0) {
					vc.pinned = false;
					Parachains::set(para_id, &pi);
					return;
				}
			}
			if let Some((vc, _)) = &mut pi.pending_upgrade {
				if vc.code_ref.is(&hash, len.0) {
					vc.pinned = false;
					Parachains::set(para_id, &pi);
					return;
				}
			}
			let out = state_balance::remove_referencer(para_id, &hash, len.0, now);
			logs.extend(out.log);
		},

		UpwardMessage::SetKV { key, value } => {
			if let Err(log) = state_balance::apply_set_kv(origin, &key, &value) {
				logs.push(log);
			}
		},

		UpwardMessage::RemoveKV { para_id, key } => {
			if Parachains::is_live(para_id) {
				state_balance::apply_remove_kv(para_id, &key);
			}
		},

		UpwardMessage::TransferOut { dest, amount, memo } => {
			transfers::transfer_out(dest, amount.0, &memo, logs)
		},

		UpwardMessage::AssignCore { core, queue, new_assigner, jam_slot } => {
			assigns::schedule(now, service_id, core, queue, new_assigner, jam_slot)
		},

		UpwardMessage::SetValidatorKeys { keys, is_last } => {
			validator_keys::apply(keys, is_last, logs)
		},

		UpwardMessage::ConsumeTransfersUpTo(slot) => transfers::consume_up_to(slot),

		UpwardMessage::UpgradeService { code_hash, len: _, min_item_gas, min_memo_gas } => {
			// §5.4: forward to JAM `upgrade` only when the new code's preimage is
			// actually provided — a solicited-but-unprovided registry entry is
			// not enough (SPEC_GAPS #5/#16).
			// FIXME: consensus-critical — JAM `upgrade` does not validate that
			// the hash decodes to a well-formed service blob (SPEC_GAPS #5).
			if is_available(&code_hash) {
				upgrade(&CodeHash(code_hash), min_item_gas, min_memo_gas);
			} else {
				logs.push(AccumulateLog::ServiceUpgradePreimageMissing { code_hash });
			}
		},

		UpwardMessage::ParachainSetHead { para_id, new_head } => {
			management::set_head(para_id, new_head)
		},

		UpwardMessage::ParachainSetValidationCode {
			para_id,
			new_validation_code_hash,
			new_validation_code_len,
		} => management::set_validation_code(
			para_id,
			new_validation_code_hash,
			new_validation_code_len.0,
			now,
			logs,
		),

		UpwardMessage::ParachainCleanUp(para_id) => management::clean_up(para_id, now, logs),

		UpwardMessage::ParachainSetStateBalance { para_id, new_total } => {
			management::set_state_balance(para_id, new_total.0, logs)
		},
	}
}
