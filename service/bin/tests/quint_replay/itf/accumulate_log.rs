//! Decode supported Accumulate events without silently accepting unknown variants.

use codec::Compact;
use parachain_service::state::log::AccumulateLog;
use serde_json::Value;

use super::{
	codex::Codex,
	replay::{field, integer, variant},
};

pub(super) fn accumulate_log(value: &Value, codex: &mut Codex) -> Result<AccumulateLog, String> {
	let (tag, value) = variant(value)?;
	match tag {
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
