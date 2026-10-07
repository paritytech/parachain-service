//! Decode supported Accumulate events without silently accepting unknown variants.

use codec::Compact;
use parachain_service::state::log::{
	AccumulateLog, InsufficientBalanceReason, ServiceCreationResult, ServiceEjectError,
	ServiceSolicitError, ServiceStoreError, ServiceSupervisorError, StateBalanceRejection,
	TransferError,
};
use parachain_service_core::types::ValidationCodeHash;
use serde_json::Value;

use super::{
	codex::Codex,
	replay::{bounded_integer, field, integer, para_id, variant},
};

pub(super) fn accumulate_log(value: &Value, codex: &mut Codex) -> Result<AccumulateLog, String> {
	let (tag, value) = variant(value)?;
	match tag {
		"InvalidCodeHashAcc" => {
			let value = integer(field(value, "vchBytes")?)?;
			let (_, hash, _) =
				codex.preimages().into_iter().find(|(known, _, _)| *known == value).ok_or_else(
					|| format!("invalid code hash {value} has no established length"),
				)?;
			Ok(AccumulateLog::InvalidCodeHash { hash: ValidationCodeHash(hash) })
		},
		"InsufficientStateBalance" => {
			let (reason, payload) = variant(value)?;
			match reason {
				"FromSolicit" => {
					let len = u32::try_from(integer(field(payload, "len")?)?)
						.map_err(|_| "FromSolicit length out of range")?;
					let hash =
						codex.hash(integer(field(field(payload, "hash")?, "hashBytes")?)?, len)?;
					Ok(AccumulateLog::InsufficientStateBalance {
						reason: InsufficientBalanceReason::Solicit { hash, len: Compact(len) },
					})
				},
				"FromSetKV" => Ok(AccumulateLog::InsufficientStateBalance {
					reason: InsufficientBalanceReason::SetKV {
						key_hash: codex.kv_key_hash(integer(field(
							field(payload, "keyHash")?,
							"kvKeyBytes",
						)?)?)?,
					},
				}),
				other => Err(format!("unsupported insufficient balance reason {other}")),
			}
		},
		"StateBalanceUpdateRejected" => {
			let (reason, payload) = variant(field(value, "reason")?)?;
			let reason = match reason {
				"BelowUsed" => StateBalanceRejection::BelowUsed {
					current_total: Compact(bounded_integer::<u64>(
						field(payload, "currentTotal")?,
						"currentTotal",
					)?),
					current_used: Compact(bounded_integer::<u64>(
						field(payload, "currentUsed")?,
						"currentUsed",
					)?),
				},
				"ParachainIsDeregistering" => StateBalanceRejection::ParachainIsDeregistering,
				other => return Err(format!("unsupported state balance rejection {other}")),
			};
			Ok(AccumulateLog::StateBalanceUpdateRejected {
				para_id: para_id(field(value, "paraId")?, codex)?,
				attempted: Compact(bounded_integer::<u64>(
					field(value, "attempted")?,
					"attempted",
				)?),
				reason,
			})
		},
		"ServiceUpgradePreimageMissing" => {
			let hash = integer(field(field(value, "codeHash")?, "hashBytes")?)?;
			Ok(AccumulateLog::ServiceUpgradePreimageMissing {
				code_hash: codex
					.known_hash(hash)
					.ok_or("service upgrade error hash has no mapped preimage")?,
			})
		},
		"ServiceCreation" => {
			let (tag, payload) = variant(field(value, "result")?)?;
			let result = match tag {
				"Created" => {
					ServiceCreationResult::Created(super::assignments::service_id(payload)?)
				},
				"CannotAfford" => ServiceCreationResult::CannotAfford,
				"IdTaken" => ServiceCreationResult::IdTaken,
				other => return Err(format!("unsupported creation result {other}")),
			};
			Ok(AccumulateLog::ServiceCreation {
				id: Compact(bounded_integer::<u64>(field(value, "id")?, "creation id")?),
				result,
			})
		},
		"ServiceSolicitFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"SolicitUnknownService" => ServiceSolicitError::UnknownService,
				"SolicitNotSupervised" => ServiceSolicitError::NotSupervised,
				other => return Err(format!("unsupported service solicit error {other}")),
			};
			Ok(AccumulateLog::ServiceSolicitFailed {
				service: super::assignments::service_id(field(value, "service")?)?,
				error,
			})
		},
		"ServiceStoreFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"StoreUnknownService" => ServiceStoreError::UnknownService,
				"StoreNotSupervised" => ServiceStoreError::NotSupervised,
				other => return Err(format!("unsupported service store error {other}")),
			};
			Ok(AccumulateLog::ServiceStoreFailed {
				service: super::assignments::service_id(field(value, "service")?)?,
				error,
			})
		},
		"ServiceSupervisorFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"HandoffUnknownService" => ServiceSupervisorError::UnknownService,
				"HandoffUnknownNewSupervisor" => ServiceSupervisorError::UnknownNewSupervisor,
				"HandoffNotSupervised" => ServiceSupervisorError::NotSupervised,
				other => return Err(format!("unsupported service supervisor error {other}")),
			};
			Ok(AccumulateLog::ServiceSupervisorFailed {
				service: super::assignments::service_id(field(value, "service")?)?,
				error,
			})
		},
		"ServiceEjectFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"TargetIsSelf" => ServiceEjectError::TargetIsSelf,
				"EjectUnknownService" => ServiceEjectError::UnknownService,
				"EjectNotSupervised" => ServiceEjectError::NotSupervised,
				other => return Err(format!("unsupported ejection error {other}")),
			};
			Ok(AccumulateLog::ServiceEjectFailed {
				service: super::assignments::service_id(field(value, "service")?)?,
				error,
			})
		},
		"TransferFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"UnknownSource" => TransferError::UnknownSource,
				"UnknownDestination" => TransferError::UnknownDestination,
				"SourceNotSupervised" => TransferError::SourceNotSupervised,
				"DestinationNotSupervised" => TransferError::DestinationNotSupervised,
				"GasBelowDestinationMinimum" => TransferError::GasBelowDestinationMinimum,
				"InsufficientServiceBalance" => TransferError::InsufficientServiceBalance,
				other => return Err(format!("unsupported transfer error {other}")),
			};
			Ok(AccumulateLog::TransferFailed {
				id: Compact(bounded_integer::<u64>(field(value, "id")?, "transfer id")?),
				error,
			})
		},
		"DesignateRejected" => Ok(AccumulateLog::DesignateRejected {
			len: Compact(bounded_integer::<u32>(field(value, "len")?, "designation length")?),
		}),
		"StagedValidatorKeysOverflow" => Ok(AccumulateLog::StagedValidatorKeysOverflow),
		"TooMuchStateHeld" => Ok(AccumulateLog::TooMuchStateHeld),
		"CoreNotAssignable" => Ok(AccumulateLog::CoreNotAssignable {
			core: bounded_integer(field(value, "core")?, "assignment core")?,
		}),
		"CodeUpgradeNotAvailable" | "CodeUpgradeNotAnnounced" | "CanNotRemoveCode" => {
			let len = u32::try_from(integer(field(value, "len")?)?)
				.map_err(|_| format!("{tag} length out of range"))?;
			let hash = codex.hash(integer(field(field(value, "hash")?, "hashBytes")?)?, len)?;
			let len = Compact(len);
			Ok(match tag {
				"CodeUpgradeNotAvailable" => AccumulateLog::CodeUpgradeNotAvailable { hash, len },
				"CodeUpgradeNotAnnounced" => AccumulateLog::CodeUpgradeNotAnnounced { hash, len },
				_ => AccumulateLog::CanNotRemoveCode { hash, len },
			})
		},
		"ForgetAgainAt" => {
			let len = u32::try_from(integer(field(value, "len")?)?)
				.map_err(|_| "ForgetAgainAt length out of range")?;
			let hash = codex.hash(integer(field(field(value, "hash")?, "hashBytes")?)?, len)?;
			let due = u32::try_from(integer(field(value, "due")?)?)
				.map_err(|_| "ForgetAgainAt deadline out of range")?;
			Ok(AccumulateLog::ForgetAgainAt { hash, len: Compact(len), due })
		},
		other => Err(format!("unsupported accumulate log event {other}")),
	}
}
