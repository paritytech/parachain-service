use jam_node::vm::Storage;
use parachain_service::state::{
	para_info::ParaInfo, preimage_registry::PreimageEntry, storage_key, Tag,
};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_core::types::ValidationCodeRef;
use serde_json::Value;

use super::{codex::Codex, replay::*};
use crate::common::set_state;

/// Seed the Rust service state represented by ITF frame zero.
pub fn seed(storage: &mut Storage, frame: &Value, codex: &mut Codex) -> Result<(), String> {
	for (para_value, info_value) in map_entries(field(field(frame, "svc")?, "parachains")?)? {
		let para = para_id(para_value, codex)?;
		let active_validation_code = option_code_ref(info_value, "validationCode", codex)?;
		let announced_upgrade = option_code_ref(info_value, "announcedUpgrade", codex)?;
		let info = ParaInfo {
			head_data: Codex::head(integer(field(info_value, "headData")?)?)?,
			validation_code: active_validation_code,
			announced_upgrade,
			total_state_balance: bounded_integer::<u64>(
				field(info_value, "totalStateBalance")?,
				"totalStateBalance",
			)?,
			used_state_balance: bounded_integer::<u64>(
				field(info_value, "usedStateBalance")?,
				"usedStateBalance",
			)?,
			is_deregistering: boolean(field(info_value, "isDeregistering")?)?,
		};
		set_state(storage, &storage_key(Tag::Parachains, &para), &info);
	}

	for (key, entry) in map_entries(field(field(frame, "svc")?, "preimageRegistry")?)? {
		let key = tuple(key)?;
		let len = bounded_integer::<u32>(&key[1], "preimage length")?;
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
		let len = bounded_integer::<u32>(&key[1], "preimage length")?;
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
	super::assignments::seed(storage, field(frame, "svc")?)?;
	storage.commit();
	Ok(())
}

/// Decode an `Option<ValidationCodeRef>` variant. Fixtures predating the
/// two-phase upgrade lifecycle omit `announcedUpgrade` entirely; they carry no
/// announcement.
pub fn option_code_ref(
	value: &Value,
	name: &str,
	codex: &mut Codex,
) -> Result<Option<ValidationCodeRef>, String> {
	let Some(option) = value.get(name) else {
		return Ok(None);
	};
	match variant(option)? {
		("None", _) => Ok(None),
		("Some", code) => Ok(Some(validation_code(code, codex)?)),
		(tag, _) => Err(format!("unexpected {name} variant {tag}")),
	}
}

/// Decode a bare `{ hash, len }` code reference. Older fixtures wrapped it as
/// `{ ref: { hash, len }, pinned }`, which was dropped when the two-phase
/// upgrade lifecycle removed the pinned bit.
pub fn validation_code(value: &Value, codex: &mut Codex) -> Result<ValidationCodeRef, String> {
	let reference = value.get("ref").unwrap_or(value);
	codex.validation_code(
		integer(field(field(reference, "hash")?, "vchBytes")?)?,
		integer(field(reference, "len")?)?,
	)
}
