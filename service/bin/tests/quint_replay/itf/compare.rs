use std::collections::{BTreeMap, BTreeSet};

use jam_node::vm::Storage;
use parachain_service::state::{
	log::{LogEntry, ParachainLog, StoredAuthTrace},
	para_info::ParaInfo,
	preimage_registry::PreimageEntry,
	storage_key, Tag,
};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_interface::types::Hash;
use serde_json::Value;

use super::{codex::Codex, refine_log::refine_log, replay::*, seed::validation_code};
use crate::common::get_state;

/// Compare every field of every Quint parachain record with Rust storage.
pub fn state(
	storage: &Storage,
	expected: &Value,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	let svc = field(expected, "svc")?;
	let mut expected_paras = BTreeSet::new();
	for (para_value, value) in map_entries(field(field(expected, "svc")?, "parachains")?)? {
		let para = para_id(para_value, codex)?;
		expected_paras.insert(para);
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
	for para in codex.paras() {
		let present: Option<ParaInfo> = get_state(storage, &storage_key(Tag::Parachains, &para));
		if present.is_some() != expected_paras.contains(&para) {
			return Err(format!(
				"frame {frame}: svc.parachains registered set differs at para {}",
				para.0
			));
		}
	}

	service_code_hash(storage, svc, codex, frame)?;
	parachain_logs(storage, svc, codex, frame)?;
	preimages(storage, svc, codex, frame)?;
	super::compare_storage::state(storage, svc, codex, frame)?;
	Ok(())
}

fn parachain_logs(
	storage: &Storage,
	svc: &Value,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	let mut expected = BTreeMap::new();
	for (para_value, log_value) in map_entries(field(svc, "parachainLog")?)? {
		let para = para_id(para_value, codex)?;
		let entries = log_value.as_array().ok_or("parachainLog value must be a list")?;
		let mut log = ParachainLog::with_capacity(entries.len());
		for entry in entries {
			let pair = tuple(entry)?;
			if pair.len() != 2 {
				return Err("parachainLog entry must contain a timeslot and LogEntry".into());
			}
			let slot = u32::try_from(integer(&pair[0])?)
				.map_err(|_| "parachainLog timeslot out of range")?;
			let (tag, value) = variant(&pair[1])?;
			let entry = match tag {
				"RefineLogEntry" => {
					let error = refine_log(field(value, "error")?)?;
					let trace = Codex::auth_trace(integer(field(value, "authTrace")?)?)?;
					let auth_trace: StoredAuthTrace = trace.0.try_into().map_err(|_| {
						"stored auth trace exceeds the 256-byte service limit".to_string()
					})?;
					LogEntry::Refine { error, auth_trace }
				},
				other => return Err(format!("unsupported parachain log entry {other}")),
			};
			log.push((slot, entry));
		}
		if expected.insert(para, log).is_some() {
			return Err(format!("duplicate svc.parachainLog key for para {}", para.0));
		}
	}

	let paras = codex.paras().collect::<Vec<_>>();
	for para in paras {
		let actual: ParachainLog =
			get_state(storage, &storage_key(Tag::ParachainLog, &para)).unwrap_or_default();
		let expected = expected.get(&para).cloned().unwrap_or_default();
		if actual != expected {
			return Err(format!(
				"frame {frame}: svc.parachainLog[{}] differs; Quint={expected:?}; Rust={actual:?}",
				para.0
			));
		}
	}
	Ok(())
}

fn service_code_hash(
	storage: &Storage,
	svc: &Value,
	codex: &Codex,
	frame: usize,
) -> Result<(), String> {
	let expected = integer(field(field(svc, "serviceCodeHash")?, "hashBytes")?)?;
	let actual = storage
		.service(MOCK_SERVICE_ID)
		.ok_or_else(|| format!("frame {frame}: parachain service missing"))?
		.code_hash
		.0;
	let actual = codex.hash_int(actual).map_err(|error| {
		format!("frame {frame}: svc.serviceCodeHash differs; Quint={expected}; Rust={error}")
	})?;
	if actual != expected {
		return Err(format!(
			"frame {frame}: svc.serviceCodeHash differs; Quint={expected}; Rust={actual}"
		));
	}
	Ok(())
}

fn preimages(
	storage: &Storage,
	svc: &Value,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	let mut expected_registry = BTreeMap::new();
	for (key, entry) in map_entries(field(svc, "preimageRegistry")?)? {
		let (abstract_hash, hash, len) = preimage_key(key, codex)?;
		let referencers = set_values(field(entry, "referencers")?)?
			.iter()
			.map(|value| para_id(value, codex))
			.collect::<Result<BTreeSet<_>, _>>()?;
		if expected_registry
			.insert((abstract_hash, hash, len), PreimageEntry { referencers })
			.is_some()
		{
			return Err("duplicate svc.preimageRegistry key".into());
		}
	}

	let mut expected_status = BTreeMap::new();
	for (key, status) in map_entries(field(svc, "preimageStatus")?)? {
		let key = preimage_key(key, codex)?;
		if expected_status.insert(key, RequestStatus::from_quint(status)?).is_some() {
			return Err("duplicate svc.preimageStatus key".into());
		}
	}

	let registry_keys = expected_registry.keys().copied().collect::<BTreeSet<_>>();
	let status_keys = expected_status.keys().copied().collect::<BTreeSet<_>>();
	if registry_keys != status_keys {
		return Err(format!(
			"frame {frame}: Quint preimageRegistry and preimageStatus keys differ"
		));
	}

	// Abstract hash zero is the fixture's boot service code. Its JAM request is
	// host setup, not part of the modeled Parachain Service preimage registry.
	for (abstract_hash, hash, len) in codex.preimages().into_iter().filter(|v| v.0 != 0) {
		let key = (abstract_hash, hash, len);
		let actual_registry: Option<PreimageEntry> =
			get_state(storage, &storage_key(Tag::PreimageRegistry, &(hash, len)));
		if actual_registry.as_ref() != expected_registry.get(&key) {
			return Err(format!(
				"frame {frame}: svc.preimageRegistry[({abstract_hash}, {len})] differs; \
				 Quint={:?}; Rust={actual_registry:?}",
				expected_registry.get(&key)
			));
		}

		let actual_status = RequestStatus::from_rust(storage, hash, len);
		let expected_status = expected_status.get(&key).cloned().unwrap_or(RequestStatus::Absent);
		if actual_status != expected_status {
			return Err(format!(
				"frame {frame}: svc.preimageStatus[({abstract_hash}, {len})] differs; \
				 Quint={expected_status:?}; Rust={actual_status:?}"
			));
		}
	}
	Ok(())
}

fn preimage_key(value: &Value, codex: &mut Codex) -> Result<(i128, Hash, u32), String> {
	let key = tuple(value)?;
	if key.len() != 2 {
		return Err("preimage key must contain hash and length".into());
	}
	let abstract_hash = integer(field(&key[0], "hashBytes")?)?;
	let len = u32::try_from(integer(&key[1])?).map_err(|_| "preimage length out of range")?;
	let hash = codex.hash(abstract_hash, len)?;
	Ok((abstract_hash, hash, len))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RequestStatus {
	Absent,
	Unprovided,
	Provided,
	Unrequested(u32),
	Rerequested(u32),
}

impl RequestStatus {
	fn from_quint(value: &Value) -> Result<Self, String> {
		let (tag, value) = variant(value)?;
		let slot =
			|| u32::try_from(integer(value)?).map_err(|_| format!("{tag} slot out of range"));
		match tag {
			"Unprovided" => Ok(Self::Unprovided),
			"Provided" => Ok(Self::Provided),
			"Unrequested" => Ok(Self::Unrequested(slot()?)),
			"Rerequested" => Ok(Self::Rerequested(slot()?)),
			_ => Err(format!("unexpected preimageStatus variant {tag}")),
		}
	}

	fn from_rust(storage: &Storage, hash: Hash, len: u32) -> Self {
		let Some(request) = storage.lookup_request(MOCK_SERVICE_ID, hash, len) else {
			return Self::Absent;
		};
		match request.0.as_slice() {
			[] => Self::Unprovided,
			[_since] => Self::Provided,
			[_since, until] => Self::Unrequested(*until),
			[_since, until, _since_again] => Self::Rerequested(*until),
			_ => unreachable!("JAM request history contains at most three slots"),
		}
	}
}

#[cfg(test)]
mod tests {
	use jam_types::CodeHash;
	use parachain_service::work_digest::RefineLog;

	use super::*;
	use crate::common::{fresh_storage, set_state};

	fn fixture() -> Value {
		serde_json::from_str(include_str!(
			"../../fixtures/quint/staleParentCandidateRejectedTest.itf.json"
		))
		.unwrap()
	}

	fn seeded(frame: &Value) -> (Storage, Codex) {
		let mut codex = Codex::default();
		let storage =
			fresh_storage(|storage| super::super::seed::seed(storage, frame, &mut codex).unwrap());
		(storage, codex)
	}

	#[test]
	fn preimage_registry_diff_errors() {
		let fixture = fixture();
		let frame = &fixture["states"][0];
		let (mut storage, mut codex) = seeded(frame);
		let hash = codex.hash(1, 65_536).unwrap();
		set_state(
			&mut storage,
			&storage_key(Tag::PreimageRegistry, &(hash, 65_536u32)),
			&PreimageEntry::default(),
		);

		let error = state(&storage, frame, &mut codex, 0).unwrap_err();
		assert!(error.contains("svc.preimageRegistry[(1, 65536)] differs"));
	}

	#[test]
	fn retained_parachain_errors() {
		let fixture = fixture();
		let frame = &fixture["states"][0];
		let (mut storage, mut codex) = seeded(frame);
		let para = codex.register_para(99).unwrap();
		let info = get_state::<ParaInfo>(
			&storage,
			&storage_key(Tag::Parachains, &Codex::para_id(1).unwrap()),
		)
		.unwrap();
		set_state(&mut storage, &storage_key(Tag::Parachains, &para), &info);

		let error = state(&storage, frame, &mut codex, 0).unwrap_err();
		assert!(error.contains("registered set differs at para 99"));
	}

	#[test]
	fn preimage_status_diff_errors() {
		let fixture = fixture();
		let frame = &fixture["states"][0];
		let (mut storage, mut codex) = seeded(frame);
		let blob = Codex::blob(1, 65_536).unwrap();
		storage.provide(0, MOCK_SERVICE_ID, &blob).unwrap();
		storage.commit();

		let error = state(&storage, frame, &mut codex, 0).unwrap_err();
		assert!(error.contains("svc.preimageStatus[(1, 65536)] differs"));
	}

	#[test]
	fn parachain_log_diff_errors() {
		let fixture = fixture();
		let frame = &fixture["states"][0];
		let (mut storage, mut codex) = seeded(frame);
		let para = Codex::para_id(1).unwrap();
		let log = vec![(
			0,
			LogEntry::Refine {
				error: RefineLog::InvalidCodeHash,
				auth_trace: StoredAuthTrace::default(),
			},
		)];
		set_state(&mut storage, &storage_key(Tag::ParachainLog, &para), &log);

		let error = state(&storage, frame, &mut codex, 0).unwrap_err();
		assert!(error.contains("svc.parachainLog[1] differs"));
	}

	#[test]
	fn request_status_variants_work() {
		let unit = serde_json::json!({ "#tup": [] });
		let slot = serde_json::json!({ "#bigint": "7" });
		for (tag, value, expected) in [
			("Unprovided", &unit, RequestStatus::Unprovided),
			("Provided", &unit, RequestStatus::Provided),
			("Unrequested", &slot, RequestStatus::Unrequested(7)),
			("Rerequested", &slot, RequestStatus::Rerequested(7)),
		] {
			let value = serde_json::json!({ "tag": tag, "value": value });
			assert_eq!(RequestStatus::from_quint(&value).unwrap(), expected);
		}
	}

	#[test]
	fn rust_request_status_variants_work() {
		let mut storage = fresh_storage(|_| {});
		let blob = Codex::blob(7, 16).unwrap();
		let hash = jam_std_common::hash_raw(&blob);

		assert_eq!(RequestStatus::from_rust(&storage, hash, 16), RequestStatus::Absent);
		storage.solicit(1, MOCK_SERVICE_ID, hash, 16).unwrap();
		storage.commit();
		assert_eq!(RequestStatus::from_rust(&storage, hash, 16), RequestStatus::Unprovided);
		storage.provide(2, MOCK_SERVICE_ID, &blob).unwrap();
		storage.commit();
		assert_eq!(RequestStatus::from_rust(&storage, hash, 16), RequestStatus::Provided);
		storage.forget(3, MOCK_SERVICE_ID, hash, 16).unwrap();
		storage.commit();
		assert_eq!(RequestStatus::from_rust(&storage, hash, 16), RequestStatus::Unrequested(3));
		storage.solicit(4, MOCK_SERVICE_ID, hash, 16).unwrap();
		storage.commit();
		assert_eq!(RequestStatus::from_rust(&storage, hash, 16), RequestStatus::Rerequested(3));
	}

	#[test]
	fn service_code_hash_diff_errors() {
		let fixture = fixture();
		let frame = &fixture["states"][0];
		let (mut storage, mut codex) = seeded(frame);
		let mut service = storage.service(MOCK_SERVICE_ID).unwrap();
		service.code_hash = CodeHash([0x55; 32]);
		storage.set_service(MOCK_SERVICE_ID, &service);

		let error = state(&storage, frame, &mut codex, 0).unwrap_err();
		assert!(error.contains("svc.serviceCodeHash differs"));
	}
}
