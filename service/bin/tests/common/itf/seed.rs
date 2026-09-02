use jam_node::vm::Storage;
use parachain_service::state::{
	para_info::{ParaInfo, ValidationCode},
	preimage_registry::PreimageEntry,
	storage_key, Tag,
};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use serde_json::Value;

use super::{codex::Codex, replay::*};
use crate::common::set_state;

/// Seed the Rust service state represented by ITF frame zero.
pub fn seed(storage: &mut Storage, frame: &Value, codex: &mut Codex) -> Result<(), String> {
	for (para_value, info_value) in map_entries(field(field(frame, "svc")?, "parachains")?)? {
		let para = para_id(para_value, codex)?;
		let active_validation_code = match variant(field(info_value, "validationCode")?)? {
			("None", _) => None,
			("Some", code) => Some(validation_code(code, codex)?),
			(tag, _) => return Err(format!("unexpected validationCode variant {tag}")),
		};
		let pending_upgrade = match variant(field(info_value, "pendingUpgrade")?)? {
			("None", _) => None,
			("Some", pair) => {
				let pair = tuple(pair)?;
				if pair.len() != 2 {
					return Err("pendingUpgrade must contain code and deadline".into());
				}
				Some((validation_code(&pair[0], codex)?, integer(&pair[1])? as u32))
			},
			(tag, _) => return Err(format!("unexpected pendingUpgrade variant {tag}")),
		};
		let info = ParaInfo {
			head_data: Codex::head(integer(field(info_value, "headData")?)?)?,
			validation_code: active_validation_code,
			pending_upgrade,
			total_state_balance: integer(field(info_value, "totalStateBalance")?)? as u64,
			used_state_balance: integer(field(info_value, "usedStateBalance")?)? as u64,
			is_deregistering: boolean(field(info_value, "isDeregistering")?)?,
		};
		set_state(storage, &storage_key(Tag::Parachains, &para), &info);
	}

	for (key, entry) in map_entries(field(field(frame, "svc")?, "preimageRegistry")?)? {
		let key = tuple(key)?;
		let len = integer(&key[1])? as u32;
		let hash = codex.hash(integer(field(&key[0], "hashBytes")?)?, len)?;
		let referencers = set_values(field(entry, "referencers")?)?
			.iter()
			.map(|value| para_id(value, codex))
			.collect::<Result<_, _>>()?;
		set_state(
			storage,
			&storage_key(Tag::PreimageRegistry, &(hash, len)),
			&PreimageEntry { referencers },
		);
	}

	for (key, status) in map_entries(field(field(frame, "svc")?, "preimageStatus")?)? {
		let key = tuple(key)?;
		let len = integer(&key[1])? as u32;
		let hash = codex.hash(integer(field(&key[0], "hashBytes")?)?, len)?;
		match variant(status)?.0 {
			"Unprovided" => {
				storage
					.solicit(0, MOCK_SERVICE_ID, hash, len)
					.map_err(|_| "failed to seed unprovided preimage")?;
			},
			other => return Err(format!("initial preimage status {other} is not supported")),
		}
	}
	storage.commit();
	Ok(())
}

pub fn validation_code(value: &Value, codex: &mut Codex) -> Result<ValidationCode, String> {
	let reference = field(value, "ref")?;
	Ok(ValidationCode {
		code_ref: codex.validation_code(
			integer(field(field(reference, "hash")?, "vchBytes")?)?,
			integer(field(reference, "len")?)?,
		)?,
		pinned: boolean(field(value, "pinned")?)?,
	})
}
