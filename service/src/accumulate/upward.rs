//! Replay of the upward messages carried in a work digest (spec §5.1 step 6).
//!
//! The PVF emits these operations through `send_upward_message` (§4.3). Refine
//! enforces their origin restrictions (D-2), which are re-checked package-wide
//! before replay starts (see `package.rs`).

use crate::{
	accumulate::{assigns, code_upgrades, foreign_services, management, transfers, validator_keys},
	head_commitment::HeadTracker,
	state::{log::AccumulateLog, para_info::Parachains},
	state_balance,
};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::{is_available, my_info, upgrade};
use jam_types::{CodeHash, ServiceId, Slot};
use parachain_service_core::{
	types::{ParaId, Timeslot, ASSET_HUB_PARA_ID},
	upward_message::{CodeUpgradePhase, Target, UpwardMessage},
};

/// Apply one upward message emitted by `origin`'s PVF. Log entries are batched
/// into `logs` and appended to the origin's `parachain_log` by the caller.
pub fn apply(
	now: Slot,
	service_id: ServiceId,
	origin: ParaId,
	lookup_anchor: Timeslot,
	message: UpwardMessage,
	logs: &mut Vec<AccumulateLog>,
	heads: &mut HeadTracker,
) {
	match message {
		UpwardMessage::RequestCodeUpgrade { hash, len, phase } => match phase {
			CodeUpgradePhase::Announcement => {
				code_upgrades::announce_code_upgrade(origin, hash, len.0, lookup_anchor, logs)
			},
			CodeUpgradePhase::Apply => code_upgrades::apply_code_upgrade(origin, hash, len.0, logs),
		},

		UpwardMessage::Solicit { target: Target::Parachain(target), hash, len } => {
			// `target` names who is charged; only the Coretime chain may name a
			// para other than itself (§6.1), and a dead target is a no-op.
			if Parachains::get(target).is_none_or(|pi| pi.is_deregistering) {
				return;
			}
			if let Err(log) = state_balance::add_referencer(target, &hash, len.0) {
				logs.push(log);
			}
		},

		UpwardMessage::Forget { target: Target::Service(service), .. } |
		UpwardMessage::RemoveServiceStorage { service, .. } => foreign_services::store_op(service, logs),

		UpwardMessage::Solicit { target: Target::Service(service), .. } => {
			foreign_services::solicit(service, logs)
		},

		UpwardMessage::EjectService { service } => {
			foreign_services::eject(service, service_id, logs)
		},

		UpwardMessage::SetServiceSupervisor { service, new_supervisor } => {
			foreign_services::set_supervisor(service, new_supervisor, logs)
		},

		UpwardMessage::CreateService(args) => foreign_services::create(args, logs),

		UpwardMessage::Forget { target: Target::Parachain(para_id), hash, len } => {
			// §5.4: expunging the running service code would prevent future accumulation.
			if para_id == ASSET_HUB_PARA_ID && hash == my_info().code_hash.0 {
				return;
			}
			// `para_id` names whose reference is released (Coretime may name any
			// para, §6.4); a dead target is a no-op.
			let Some(pi) = Parachains::get(para_id) else { return };
			// The target's active or announced validation code: the forget is
			// refused, so the referencer and the balance both stay (§5.2). This
			// is what stops a forced call from stripping running validation code.
			let is_validation_code =
				pi.validation_code.as_ref().is_some_and(|vc| vc.is(&hash, len.0)) ||
					pi.announced_upgrade.as_ref().is_some_and(|vc| vc.is(&hash, len.0));
			if is_validation_code {
				logs.push(AccumulateLog::CanNotForgetValidationCode { hash, len: len.0.into() });
				return;
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
			if Parachains::get(para_id).is_some_and(|pi| !pi.is_deregistering) {
				state_balance::apply_remove_kv(para_id, &key);
			}
		},

		UpwardMessage::TransferOut(args) => transfers::transfer_out(service_id, args, logs),

		UpwardMessage::AssignCore { core, queue, new_assigner, jam_slot } => {
			assigns::schedule(now, service_id, core, queue, new_assigner, jam_slot, logs)
		},

		UpwardMessage::SetValidatorKeys { keys, is_last } => {
			validator_keys::apply(keys, is_last, logs)
		},

		UpwardMessage::CleanUpBucketsUpTo(id) => transfers::clean_up_buckets_up_to(id),

		UpwardMessage::UpgradeService { code_hash, len: _, min_acc_gas, min_memo_gas } => {
			// §5.4: forward to JAM `upgrade` only when the new code's preimage is
			// actually provided — a solicited-but-unprovided registry entry is
			// not enough.
			// FIXME: consensus-critical — JAM `upgrade` does not validate that
			// the hash decodes to a well-formed service blob.
			if is_available(&code_hash) {
				upgrade(&CodeHash(code_hash), min_acc_gas, min_memo_gas);
			} else {
				logs.push(AccumulateLog::ServiceUpgradePreimageMissing { code_hash });
			}
		},

		UpwardMessage::ParachainSetHead { para_id, new_head } => {
			management::set_head(para_id, new_head, heads, logs)
		},

		UpwardMessage::ParachainSetValidationCode {
			para_id,
			new_validation_code_hash,
			new_validation_code_len,
		} => management::set_validation_code(
			para_id,
			new_validation_code_hash,
			new_validation_code_len.0,
			logs,
		),

		UpwardMessage::ParachainCleanUp(para_id) => management::clean_up(para_id, now, logs, heads),

		UpwardMessage::ParachainSetStateBalance { para_id, new_total } => {
			management::set_state_balance(para_id, new_total.0, logs, heads)
		},
	}
}
