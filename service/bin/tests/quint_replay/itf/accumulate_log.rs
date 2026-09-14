//! Decode supported Accumulate events without silently accepting unknown variants.

use codec::Compact;
use parachain_service::state::log::{AccumulateLog, InsufficientBalanceReason};
use parachain_service_interface::types::ValidationCodeHash;
use serde_json::Value;

use super::{
	codex::Codex,
	replay::{field, integer, variant},
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
				other => Err(format!("unsupported insufficient balance reason {other}")),
			}
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
