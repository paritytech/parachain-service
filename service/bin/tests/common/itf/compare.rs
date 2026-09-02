use jam_node::vm::Storage;
use parachain_service::state::{para_info::ParaInfo, storage_key, Tag};
use serde_json::Value;

use super::{codex::Codex, replay::*, seed::validation_code};
use crate::common::get_state;

/// Compare every field of every Quint parachain record with Rust storage.
pub fn state(
	storage: &Storage,
	expected: &Value,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	for (para_value, value) in map_entries(field(field(expected, "svc")?, "parachains")?)? {
		let para = para_id(para_value, codex)?;
		let actual: ParaInfo = get_state(storage, &storage_key(Tag::Parachains, &para))
			.ok_or_else(|| format!("frame {frame}: para {} missing", para.0))?;
		let expected_validation = match variant(field(value, "validationCode")?)? {
			("None", _) => None,
			("Some", code) => Some(validation_code(code, codex)?),
			(tag, _) => return Err(format!("unexpected validationCode variant {tag}")),
		};
		let expected_pending = match variant(field(value, "pendingUpgrade")?)? {
			("None", _) => None,
			("Some", pair) => {
				let pair = tuple(pair)?;
				Some((validation_code(&pair[0], codex)?, integer(&pair[1])? as u32))
			},
			(tag, _) => return Err(format!("unexpected pendingUpgrade variant {tag}")),
		};
		let checks = [
			("headData", actual.head_data == Codex::head(integer(field(value, "headData")?)?)?),
			("validationCode", actual.validation_code == expected_validation),
			("pendingUpgrade", actual.pending_upgrade == expected_pending),
			(
				"totalStateBalance",
				actual.total_state_balance == integer(field(value, "totalStateBalance")?)? as u64,
			),
			(
				"usedStateBalance",
				actual.used_state_balance == integer(field(value, "usedStateBalance")?)? as u64,
			),
			(
				"isDeregistering",
				actual.is_deregistering == boolean(field(value, "isDeregistering")?)?,
			),
		];
		if let Some((name, _)) = checks.into_iter().find(|(_, equal)| !equal) {
			return Err(format!(
				"frame {frame}: svc.parachains[{}].{name} differs; Rust={actual:?}",
				para.0
			));
		}
	}
	Ok(())
}
