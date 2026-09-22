//! Assignment value mapping and final JAM effects. The host exposes final
//! mutations per core, not a call log; repeated calls are folded in model order.
use std::collections::BTreeMap;

use jam_node::vm::{StateMutations, Storage};
use jam_std_common::Privileges;
use jam_types::AuthorizerHash;
use parachain_service::state::{assigns::PendingAssign, storage_key, Tag};
use parachain_service_bin::mock::{accumulate_context, MOCK_SERVICE_ID};
use parachain_service_core::upward_message::UpwardMessage;
use serde_json::Value;

use super::{codex::Codex, replay::*};
use crate::common::set_state;

// Quint's service is 1, the PVM mock's is 0. Swap those two IDs so foreign
// service 0 remains distinct. This mapping is scoped to assignment service IDs.
pub fn service_id(value: &Value) -> Result<u32, String> {
	let (tag, value) = variant(value)?;
	if tag != "MkServiceId" {
		return Err("expected MkServiceId".into());
	}
	let id = bounded_integer::<u32>(value, "assignment service ID")?;
	Ok(match id {
		1 => MOCK_SERVICE_ID,
		0 => 1,
		other => other,
	})
}

pub fn assigner(value: &Value) -> Result<Option<u32>, String> {
	match variant(value)? {
		("None", _) => Ok(None),
		("Some", value) => Ok(Some(service_id(value)?)),
		_ => Err("invalid assigner option".into()),
	}
}

pub fn queue(value: &Value) -> Result<Vec<AuthorizerHash>, String> {
	value
		.as_array()
		.ok_or("queue must be a list")?
		.iter()
		.map(|v| Codex::authorizer_hash(integer(field(v, "authBytes")?)?))
		.collect()
}

pub fn message(value: &Value) -> Result<UpwardMessage, String> {
	Ok(UpwardMessage::AssignCore {
		core: bounded_integer(field(value, "core")?, "assignment core")?,
		queue: queue(field(value, "queue")?)?.into_iter().map(|h| h.0).collect(),
		new_assigner: assigner(field(value, "newAssigner")?)?,
		jam_slot: bounded_integer(field(value, "jamSlot")?, "assignment slot")?,
	})
}

pub fn pending(value: &Value) -> Result<PendingAssign, String> {
	Ok(PendingAssign {
		queue: queue(field(value, "queue")?)?.into_iter().map(|h| h.0).collect(),
		assigner: assigner(field(value, "assigner")?)?,
	})
}

pub fn seed(storage: &mut Storage, svc: &Value) -> Result<(), String> {
	let due = map_entries(field(svc, "pendingAssignCores")?)?
		.into_iter()
		.map(|(core, slot)| {
			Ok((bounded_integer::<u16>(core, "core")?, bounded_integer::<u32>(slot, "due slot")?))
		})
		.collect::<Result<Vec<_>, String>>()?;
	if !due.is_empty() {
		set_state(storage, &storage_key(Tag::PendingAssignCores, &()), &due);
	}
	for (core, entry) in map_entries(field(svc, "pendingAssigns")?)? {
		let core = bounded_integer::<u16>(core, "core")?;
		set_state(storage, &storage_key(Tag::PendingAssigns, &core), &pending(entry)?);
	}
	Ok(())
}

pub fn initial_privileges(storage: Storage) -> Privileges {
	accumulate_context(storage, Vec::new(), 0).privileges
}

pub fn compare(
	current: &Value,
	mutations: &StateMutations,
	before: &Privileges,
	frame: usize,
) -> Result<(), String> {
	let result = (|| {
		let calls = field(current, "lastStepAssigns")?
			.as_array()
			.ok_or("lastStepAssigns must be a list")?;
		let mut expected = BTreeMap::new();
		let mut privileges = before.clone();
		for call in calls {
			let core = bounded_integer::<u16>(field(call, "core")?, "assignment core")?;
			if usize::from(core) >= privileges.assign.len() {
				return Err(format!("assignment core {core} exceeds JAM core count"));
			}
			let queue = queue(field(call, "queue")?)?;
			if queue.len() != jam_types::AuthQueue::default().len() {
				return Err("assignment host queue has wrong length".into());
			}
			expected.insert(core, queue);
			privileges.assign[usize::from(core)] = service_id(field(call, "assigner")?)?;
		}
		let actual: BTreeMap<_, Vec<_>> = mutations
			.auths
			.iter()
			.map(|(core, queue)| (*core, queue.iter().copied().collect()))
			.collect();
		if actual.keys().collect::<Vec<_>>() != expected.keys().collect::<Vec<_>>() {
			return Err(format!(
				"assigned cores differ; Quint={:?}; Rust={:?}",
				expected.keys().collect::<Vec<_>>(),
				actual.keys().collect::<Vec<_>>()
			));
		}
		for (core, queue) in &expected {
			if actual.get(core) != Some(queue) {
				return Err(format!("authorizer queue differs for core {core}"));
			}
		}
		let expected_privileges = (!calls.is_empty()).then_some(privileges);
		if mutations.privileges != expected_privileges {
			return Err("assignment/privilege output differs".into());
		}
		Ok(())
	})();
	result.map_err(|error: String| format!("frame {frame}: lastStepAssigns: {error}"))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::common::fresh_storage;
	use serde_json::json;

	fn n(v: u32) -> Value {
		json!({"#bigint": v.to_string()})
	}
	fn call(core: u16, owner: u32, hash: u32) -> Value {
		json!({"core": n(core.into()), "assigner": {"tag":"MkServiceId", "value":n(owner)},
            "queue": vec![json!({"authBytes": n(hash)}); jam_types::AuthQueue::default().len()]})
	}
	#[test]
	fn host_mutations_errors() {
		let before = initial_privileges(fresh_storage(|_| {}));
		let mut current = json!({"lastStepAssigns": [call(2, 1, 9), call(2, 7, 10)]});
		current["lastStepAssigns"][1]["queue"][1]["authBytes"] = n(11);
		let mut good = StateMutations::new(0);
		let mut queue = jam_types::AuthQueue::new(Codex::authorizer_hash(10).unwrap());
		queue[1] = Codex::authorizer_hash(11).unwrap();
		good.auths.insert(2, queue);
		let mut privileges = before.clone();
		privileges.assign[2] = 7;
		good.privileges = Some(privileges);
		compare(&current, &good, &before, 3).unwrap();
		for kind in 0..7 {
			let mut bad = good.clone();
			match kind {
				0 => {
					bad.auths.clear();
				},
				1 => {
					bad.auths
						.insert(2, jam_types::AuthQueue::new(Codex::authorizer_hash(9).unwrap()));
				},
				2 => {
					bad.auths.insert(3, Default::default());
				},
				3 => {
					bad.privileges = None;
				},
				4 => {
					bad.privileges.as_mut().unwrap().assign[2] = 0;
				},
				5 => {
					bad.privileges.as_mut().unwrap().bless = 99;
				},
				_ => {
					bad.auths.get_mut(&2).unwrap().swap(0, 1);
				},
			}
			assert!(compare(&current, &bad, &before, 3).unwrap_err().contains("lastStepAssigns"));
		}
		let mut reversed = current.clone();
		reversed["lastStepAssigns"].as_array_mut().unwrap().reverse();
		assert!(compare(&reversed, &good, &before, 3).is_err());
		assert!(compare(&json!({"lastStepAssigns": []}), &good, &before, 3).is_err());
	}

	#[test]
	fn assignment_service_ids_works() {
		for (model, rust) in [(0, 1), (1, MOCK_SERVICE_ID), (7, 7), (u32::MAX, u32::MAX)] {
			assert_eq!(service_id(&json!({"tag":"MkServiceId", "value": n(model)})).unwrap(), rust);
		}
	}

	#[test]
	fn malformed_assignment_values_errors() {
		let original = json!({"core": n(2), "queue": [{"authBytes": n(9)}],
            "newAssigner": {"tag":"Some", "value":{"tag":"MkServiceId", "value":n(7)}},
            "jamSlot":n(10)});
		for (pointer, value) in [
			("/core", json!({"#bigint":"65536"})),
			("/jamSlot", json!({"#bigint":"-1"})),
			("/newAssigner/value/value", json!({"#bigint":"4294967296"})),
			("/newAssigner/value/tag", json!("MkParaId")),
			("/queue/0/authBytes", json!({"#bigint":"-1"})),
		] {
			let mut changed = original.clone();
			*changed.pointer_mut(pointer).unwrap() = value;
			assert!(message(&changed).is_err(), "{pointer}");
		}
	}

	#[test]
	fn seeded_pending_assignments_works() {
		let document: Value = serde_json::from_str(include_str!(
			"../../fixtures/quint/assignments/delayed_boundary_works.itf.json"
		))
		.unwrap();
		let mut frame = document["states"][0].clone();
		for name in ["pendingAssigns", "pendingAssignCores"] {
			frame["svc"][name] = document["states"][1]["svc"][name].clone();
		}
		let mut codex = Codex::default();
		let storage = fresh_storage(|s| super::super::seed::seed(s, &frame, &mut codex).unwrap());
		super::super::compare::state(&storage, &frame, &mut codex, 0).unwrap();
	}
}
