//! Comparisons for state outside parachain records, logs, and preimages.
use std::collections::{BTreeMap, BTreeSet};

use jam_node::vm::Storage;
use jam_std_common::{ServiceKey, StorageKey};
use parachain_service::state::{assigns::PendingAssign, storage_key, Tag};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use serde_json::Value;

use super::{codex::Codex, replay::*};
use crate::common::get_state;

pub fn state(
	storage: &Storage,
	svc: &Value,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	let mut allowed = BTreeSet::new();
	let mut allow = |key: Vec<u8>| {
		let key: StorageKey = ServiceKey::Value { id: MOCK_SERVICE_ID, key: &key }.into();
		allowed.insert(key);
	};
	for (para, _) in map_entries(field(svc, "parachains")?)? {
		allow(storage_key(Tag::Parachains, &para_id(para, codex)?));
	}
	for para in codex.paras() {
		// An empty log may be represented by an absent key or an encoded empty list.
		allow(storage_key(Tag::ParachainLog, &para));
	}
	for (_, hash, len) in codex.preimages() {
		allow(storage_key(Tag::PreimageRegistry, &(hash, len)));
	}

	let mut expected_due = BTreeMap::new();
	for (core, slot) in map_entries(field(svc, "pendingAssignCores")?)? {
		let core = u16::try_from(integer(core)?).map_err(|_| "core out of range")?;
		if expected_due.insert(core, uint(slot)?).is_some() {
			return Err("duplicate pendingAssignCores key".into());
		}
	}
	let due: Vec<(u16, u32)> =
		get_state(storage, &storage_key(Tag::PendingAssignCores, &())).unwrap_or_default();
	let actual_due: BTreeMap<_, _> = due.iter().copied().collect();
	if actual_due != expected_due || actual_due.len() != due.len() {
		return Err(format!("frame {frame}: svc.pendingAssignCores differs"));
	}
	allow(storage_key(Tag::PendingAssignCores, &()));
	for (core, entry) in map_entries(field(svc, "pendingAssigns")?)? {
		let core = u16::try_from(integer(core)?).map_err(|_| "core out of range")?;
		let expected = super::assignments::pending(entry)?;
		let key = storage_key(Tag::PendingAssigns, &core);
		if get_state::<PendingAssign>(storage, &key) != Some(expected) {
			return Err(format!("frame {frame}: svc.pendingAssigns[{core}] differs"));
		}
		allow(key);
	}

	let expected_keys = field(svc, "stagedValidatorKeys")?
		.as_array()
		.ok_or("stagedValidatorKeys must be a list")?
		.iter()
		.map(|v| {
			let n = u64::try_from(integer(v)?).map_err(|_| "validator key out of range")?;
			let mut key = [0u8; 336];
			key[..8].copy_from_slice(&n.to_le_bytes());
			Ok(key)
		})
		.collect::<Result<Vec<_>, String>>()?;
	let key = storage_key(Tag::StagedValidatorKeys, &());
	let actual: Vec<[u8; 336]> = get_state(storage, &key).unwrap_or_default();
	if actual != expected_keys {
		return Err(format!("frame {frame}: svc.stagedValidatorKeys differs"));
	}
	allow(key);

	for (key, value) in map_entries(field(svc, "keyValueStorage")?)? {
		let pair = tuple(key)?;
		if pair.len() != 2 {
			return Err("KV key must contain para and bytes".into());
		}
		let para = para_id(&pair[0], codex)?;
		let key = storage_key(Tag::KeyValueStorage, &(para, bytes(&pair[1])?));
		if get_state::<Vec<u8>>(storage, &key) != Some(bytes(value)?) {
			return Err(format!("frame {frame}: svc.keyValueStorage differs"));
		}
		allow(key);
	}

	for key in super::transfers::compare(storage, svc, frame)? {
		allow(key);
	}

	// JAM hashes service keys, so their original tags cannot be recovered. Check
	// the complete allowed key set to catch extra KV entries, orphan transfer
	// buckets, and pending assignments even when the model maps are empty.
	let mut preimages = codex.preimages();
	let blob = parachain_service_bin::blob();
	preimages.push((0, jam_std_common::hash_raw(&blob), blob.len() as u32));
	for (_, hash, len) in preimages {
		allowed.insert(ServiceKey::Request { id: MOCK_SERVICE_ID, hash, len }.into());
		allowed.insert(ServiceKey::Preimage { id: MOCK_SERVICE_ID, hash }.into());
	}
	let id = MOCK_SERVICE_ID.to_le_bytes();
	for key in storage.keys() {
		if [key[0], key[2], key[4], key[6]] == id && !allowed.contains(&StorageKey::from(key)) {
			return Err(format!("frame {frame}: unexpected service storage key {key:?} (including pendingAssigns, incomingTransfers, or keyValueStorage)"));
		}
	}
	Ok(())
}

fn uint(v: &Value) -> Result<u32, String> {
	u32::try_from(integer(v)?).map_err(|_| "integer out of u32 range".into())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::common::{fresh_storage, set_state};
	use serde_json::json;

	fn svc() -> Value {
		json!({
			"parachains": {"#map": []}, "pendingAssignCores": {"#map": []},
			"pendingAssigns": {"#map": []}, "stagedValidatorKeys": [],
			"keyValueStorage": {"#map": []}, "incomingTransfers": {"#map": []},
			"incomingTransferChain": {"tag": "None", "value": {"#tup": []}}
		})
	}
	fn n(v: u32) -> Value {
		json!({"#bigint": v.to_string()})
	}

	#[test]
	fn empty_state_works() {
		state(&fresh_storage(|_| {}), &svc(), &mut Codex::default(), 0).unwrap();
	}

	#[test]
	fn unexpected_storage_errors() {
		for tag in [Tag::PendingAssigns, Tag::IncomingTransfers, Tag::KeyValueStorage] {
			let storage = fresh_storage(|s| set_state(s, &storage_key(tag, &99u32), &vec![1u8]));
			let error = state(&storage, &svc(), &mut Codex::default(), 7).unwrap_err();
			assert!(error.contains("frame 7: unexpected service storage key"), "{error}");
		}
	}

	#[test]
	fn unexpected_singletons_errors() {
		for (tag, expected) in [
			(Tag::PendingAssignCores, "pendingAssignCores"),
			(Tag::StagedValidatorKeys, "stagedValidatorKeys"),
			(Tag::IncomingTransferBuckets, "incomingTransferBuckets"),
		] {
			let storage = fresh_storage(|s| match tag {
				Tag::PendingAssignCores => {
					set_state(s, &storage_key(tag, &()), &vec![(1u16, 2u32)])
				},
				Tag::StagedValidatorKeys => set_state(s, &storage_key(tag, &()), &vec![[1u8; 336]]),
				_ => set_state(s, &storage_key(tag, &()), &1u8),
			});
			let error = state(&storage, &svc(), &mut Codex::default(), 3).unwrap_err();
			assert!(error.contains(&format!("frame 3: svc.{expected} differs")), "{error}");
		}
	}

	fn nonempty_state() -> (Storage, Value) {
		let mut expected = svc();
		expected["pendingAssignCores"] = json!({"#map": [[n(2), n(10)]]});
		expected["pendingAssigns"] = json!({"#map": [[n(2), {
			"queue": [{"authBytes": n(9)}, {"authBytes": n(10)}],
			"assigner": {"tag": "Some", "value": {"tag": "MkServiceId", "value": n(5)}}
		}]]});
		expected["stagedValidatorKeys"] = json!([n(7), n(8)]);
		expected["keyValueStorage"] = json!({"#map": [[
			{"#tup": [{"tag": "MkParaId", "value": n(3)}, [n(0), n(255)]]}, [n(42), n(43)]
		]]});
		let storage = fresh_storage(|s| {
			set_state(s, &storage_key(Tag::PendingAssignCores, &()), &vec![(2u16, 10u32)]);
			set_state(
				s,
				&storage_key(Tag::PendingAssigns, &2u16),
				&PendingAssign {
					queue: vec![
						Codex::authorizer_hash(9).unwrap().0,
						Codex::authorizer_hash(10).unwrap().0,
					],
					assigner: Some(5),
				},
			);
			let mut key = [0u8; 336];
			key[0] = 7;
			let mut second_key = key;
			second_key[0] = 8;
			set_state(s, &storage_key(Tag::StagedValidatorKeys, &()), &vec![key, second_key]);
			set_state(
				s,
				&storage_key(Tag::KeyValueStorage, &(Codex::para_id(3).unwrap(), vec![0u8, 255])),
				&vec![42u8, 43],
			);
		});
		(storage, expected)
	}

	#[test]
	fn nonempty_fields_works() {
		let (storage, expected) = nonempty_state();
		state(&storage, &expected, &mut Codex::default(), 0).unwrap();
		for field in
			["pendingAssignCores", "pendingAssigns", "stagedValidatorKeys", "keyValueStorage"]
		{
			let mut changed = expected.clone();
			changed[field] = svc()[field].clone();
			assert!(state(&storage, &changed, &mut Codex::default(), 0).is_err(), "{field}");
		}
	}

	// Start from matching nonempty storage, then corrupt only one expectation.
	fn rejects_change(pointer: &str, value: Value, field: &str) {
		let (storage, mut expected) = nonempty_state();
		state(&storage, &expected, &mut Codex::default(), 4).unwrap();
		*expected.pointer_mut(pointer).expect("existing expected field") = value;
		let error = state(&storage, &expected, &mut Codex::default(), 4).unwrap_err();
		assert!(error.contains(&format!("frame 4: svc.{field}")), "{error}");
	}

	#[test]
	fn assignment_values_errors() {
		rejects_change("/pendingAssignCores/#map/0/1", n(11), "pendingAssignCores");
		rejects_change("/pendingAssignCores/#map/0/0", n(3), "pendingAssignCores");
		rejects_change("/pendingAssigns/#map/0/1/queue/0/authBytes", n(11), "pendingAssigns[2]");
		rejects_change("/pendingAssigns/#map/0/1/assigner/value/value", n(6), "pendingAssigns[2]");
		rejects_change(
			"/pendingAssigns/#map/0/1/assigner",
			json!({"tag": "None", "value": {"#tup": []}}),
			"pendingAssigns[2]",
		);
	}

	#[test]
	fn assignment_queue_order_errors() {
		rejects_change(
			"/pendingAssigns/#map/0/1/queue",
			json!([{"authBytes": n(10)}, {"authBytes": n(9)}]),
			"pendingAssigns[2]",
		);
	}

	#[test]
	fn validator_key_value_errors() {
		rejects_change("/stagedValidatorKeys/0", n(9), "stagedValidatorKeys");
	}

	#[test]
	fn validator_key_order_errors() {
		rejects_change("/stagedValidatorKeys", json!([n(8), n(7)]), "stagedValidatorKeys");
	}

	#[test]
	fn kv_value_errors() {
		rejects_change("/keyValueStorage/#map/0/1/0", n(44), "keyValueStorage");
		rejects_change("/keyValueStorage/#map/0/1", json!([]), "keyValueStorage");
	}

	#[test]
	fn kv_byte_order_errors() {
		rejects_change("/keyValueStorage/#map/0/1", json!([n(43), n(42)]), "keyValueStorage");
		rejects_change(
			"/keyValueStorage/#map/0/0/#tup/1",
			json!([n(255), n(0)]),
			"keyValueStorage",
		);
	}

	#[test]
	fn kv_owner_errors() {
		rejects_change("/keyValueStorage/#map/0/0/#tup/0/value", n(4), "keyValueStorage");
	}

	#[test]
	fn nonempty_transfer_model_errors() {
		let mut expected = svc();
		expected["incomingTransfers"] = json!({"#map": [[n(1), {}]]});
		let error = state(&fresh_storage(|_| {}), &expected, &mut Codex::default(), 0).unwrap_err();
		assert!(error.contains("incoming bucket must be a list"), "{error}");
	}
}
