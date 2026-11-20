//! Decode supported Accumulate events without silently accepting unknown variants.

use codec::Compact;
use parachain_service::state::log::{
	AccumulateLog, ServiceCreationResult, ServiceEjectError, ServiceSolicitError,
	ServiceSupervisorError, StateBalanceRejection, TransferError,
};
use parachain_service_core::types::{MessageId, ValidationCodeHash};
use serde_json::Value;

use super::{
	codex::Codex,
	replay::{bounded_integer, field, integer, variant},
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
			Ok(AccumulateLog::InvalidCodeHash { hash: ValidationCodeHash::from(hash) })
		},
		"InsufficientStateBalance" => Ok(AccumulateLog::InsufficientStateBalance),
		"StateBalanceUpdateRejected" => {
			let (reason, payload) = variant(field(value, "reason")?)?;
			let reason = match reason {
				"BelowUsed" => StateBalanceRejection::BelowUsed {
					current_total: Compact(bounded_integer(
						field(payload, "currentTotal")?,
						"currentTotal",
					)?),
					current_used: Compact(bounded_integer(
						field(payload, "currentUsed")?,
						"currentUsed",
					)?),
				},
				"ParachainIsDeregistering" => StateBalanceRejection::ParachainIsDeregistering,
				other => return Err(format!("unsupported state balance rejection {other}")),
			};
			Ok(AccumulateLog::StateBalanceUpdateRejected { reason })
		},
		"ServiceUpgradePreimageMissing" => Ok(AccumulateLog::ServiceUpgradePreimageMissing),
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
			Ok(AccumulateLog::ServiceCreation { result })
		},
		"ServiceSolicitFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"SolicitUnknownService" => ServiceSolicitError::UnknownService,
				"SolicitNotSupervised" => ServiceSolicitError::NotSupervised,
				"TargetCannotAfford" => ServiceSolicitError::TargetCannotAfford,
				"AlreadySolicited" => ServiceSolicitError::AlreadySolicited,
				other => return Err(format!("unsupported service solicit error {other}")),
			};
			Ok(AccumulateLog::ServiceSolicitFailed { error })
		},
		"ServiceSupervisorFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"HandoffUnknownService" => ServiceSupervisorError::UnknownService,
				"HandoffUnknownNewSupervisor" => ServiceSupervisorError::UnknownNewSupervisor,
				"HandoffNotSupervised" => ServiceSupervisorError::NotSupervised,
				other => return Err(format!("unsupported service supervisor error {other}")),
			};
			Ok(AccumulateLog::ServiceSupervisorFailed { error })
		},
		"ServiceEjectFailed" => {
			let error = match variant(field(value, "error")?)?.0 {
				"TargetIsSelf" => ServiceEjectError::TargetIsSelf,
				"EjectUnknownService" => ServiceEjectError::UnknownService,
				"EjectNotSupervised" => ServiceEjectError::NotSupervised,
				"NotEmpty" => ServiceEjectError::NotEmpty,
				"CreatedThisSlot" => ServiceEjectError::CreatedThisSlot,
				other => return Err(format!("unsupported ejection error {other}")),
			};
			Ok(AccumulateLog::ServiceEjectFailed { error })
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
			Ok(AccumulateLog::TransferFailed { error })
		},
		"TransferredAmount" => Ok(AccumulateLog::TransferredAmount {
			amount: Compact(bounded_integer::<u64>(field(value, "amount")?, "transferred amount")?),
		}),
		"DesignateRejected" => Ok(AccumulateLog::DesignateRejected {
			len: Compact(bounded_integer::<u32>(field(value, "len")?, "designation length")?),
		}),
		"StagedValidatorKeysOverflow" => Ok(AccumulateLog::StagedValidatorKeysOverflow),
		"TooMuchStateHeld" => Ok(AccumulateLog::TooMuchStateHeld),
		"CoreNotAssignable" => Ok(AccumulateLog::CoreNotAssignable),
		"CodeUpgradeNotAvailable" => Ok(AccumulateLog::CodeUpgradeNotAvailable),
		"CodeUpgradeNotAnnounced" => Ok(AccumulateLog::CodeUpgradeNotAnnounced),
		"CanNotRemoveCode" => {
			let len = u32::try_from(integer(field(value, "len")?)?)
				.map_err(|_| "CanNotRemoveCode length out of range")?;
			let hash = codex.hash(integer(field(field(value, "hash")?, "hashBytes")?)?, len)?;
			Ok(AccumulateLog::CanNotRemoveCode { hash, len: Compact(len) })
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

/// Decode one `(message_id, AccumulateLog)` pair of a `LogEntry::Accumulate`.
pub(super) fn tagged_accumulate_log(
	value: &Value,
	codex: &mut Codex,
) -> Result<(MessageId, AccumulateLog), String> {
	let pair = super::replay::tuple(value)?;
	let [id, event] = pair.as_slice() else {
		return Err("Accumulate entry must be a (message_id, event) pair".into());
	};
	Ok((
		MessageId::from(bounded_integer::<u64>(id, "accumulate message id")?),
		accumulate_log(event, codex)?,
	))
}
