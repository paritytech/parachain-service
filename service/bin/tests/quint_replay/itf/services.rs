//! CreateService operands and complete account/request effects on the pinned host.
use std::collections::{BTreeMap, BTreeSet};

use jam_node::vm::{StateMutations, Storage};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_core::upward_message::{CreateServiceArgs, UpwardMessage};
use serde_json::Value;

use super::{assignments::service_id, codex::Codex, replay::*};

pub fn validate(states: &[Value]) -> Result<(), String> {
	let enabled = states.first().is_some_and(|s| s.get("replayCreations").is_some());
	for (index, state) in states.iter().enumerate() {
		if state.get("replayCreations").is_some() != enabled {
			return Err("service traces require replayCreations in every frame".into());
		}
		if let Some(creations) = state.get("replayCreations") {
			let creations = creations.as_array().ok_or("replayCreations must be a list")?;
			if index == 0 && !creations.is_empty() {
				return Err("initial replayCreations must be empty".into());
			}
		}
		let creates =
			state
				.get("lastStepWorkResults")
				.and_then(Value::as_array)
				.is_some_and(|reports| {
					reports.iter().any(|r| {
						r.pointer("/result/value/value/upwardMessages")
							.and_then(Value::as_array)
							.is_some_and(|msgs| msgs.iter().any(|m| m["tag"] == "CreateService"))
					})
				});
		if creates && (!enabled || state.get("replayTransfers").is_none()) {
			return Err("CreateService inputs require replayCreations and replayTransfers".into());
		}
	}
	Ok(())
}

pub fn args(value: &Value, codex: &mut Codex) -> Result<CreateServiceArgs, String> {
	let len = bounded_integer::<u32>(field(value, "len")?, "creation code length")?;
	Ok(CreateServiceArgs {
		code_hash: codex.hash(integer(field(field(value, "codeHash")?, "hashBytes")?)?, len)?,
		len: len.into(),
		min_item_gas: bounded_integer(field(value, "minItemGas")?, "creation item gas")?,
		min_memo_gas: bounded_integer(field(value, "minMemoGas")?, "creation memo gas")?,
		id: bounded_integer::<u64>(field(value, "id")?, "creation id")?.into(),
		desired_id: super::assignments::assigner(field(value, "desiredId")?)?,
		source_supervisor_balance: boolean(field(value, "sourceSupervisorBalance")?)?,
		new_supervisor_balance: boolean(field(value, "newSupervisorBalance")?)?,
	})
}

pub fn message(value: &Value, codex: &mut Codex) -> Result<UpwardMessage, String> {
	Ok(UpwardMessage::CreateService(args(value, codex)?))
}

pub fn effects(
	previous: &Value,
	current: &Value,
	mutations: &StateMutations,
	frame: usize,
) -> Result<(), String> {
	let before: BTreeSet<_> = map_entries(field(previous, "foreignServices")?)?
		.into_iter()
		.map(|(id, _)| service_id(id))
		.collect::<Result<_, _>>()?;
	let after: BTreeSet<_> = map_entries(field(current, "foreignServices")?)?
		.into_iter()
		.map(|(id, _)| service_id(id))
		.collect::<Result<_, _>>()?;
	let expected: BTreeSet<_> = after.difference(&before).copied().collect();
	let mut declared = BTreeSet::new();
	if let Some(creations) = current.get("replayCreations") {
		for creation in creations.as_array().ok_or("replayCreations must be a list")? {
			if !declared.insert(service_id(field(creation, "service")?)?) {
				return Err("duplicate replayCreations service".into());
			}
		}
	}
	if !before.is_subset(&after) ||
		expected != declared ||
		expected != mutations.created.iter().copied().collect() ||
		!mutations.ejected.is_empty()
	{
		return Err(format!(
			"frame {frame}: unexpected JAM provide/create/eject output: creation set differs"
		));
	}
	Ok(())
}

/// Keep checking created accounts in subsequent frames, including after an
/// attempted ejection or a transfer. Metadata is retained from each creation.
pub fn accounts(
	storage: &Storage,
	current: &Value,
	known: &mut BTreeMap<u32, (CreateServiceArgs, u32)>,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	if let Some(creations) = current.get("replayCreations") {
		for creation in creations.as_array().ok_or("replayCreations must be a list")? {
			let id = service_id(field(creation, "service")?)?;
			let args = args(field(creation, "args")?, codex)?;
			let slot = bounded_integer::<u32>(field(current, "now")?, "creation slot")?;
			if args.source_supervisor_balance || args.new_supervisor_balance {
				return Err("successful creation cannot use supervisor balances".into());
			}
			if known.insert(id, (args, slot)).is_some() {
				return Err("created service ID reused".into());
			}
		}
	}
	for (&id, (args, slot)) in known.iter() {
		let slot = *slot;
		let actual = storage.service(id).ok_or("missing created service")?;
		if actual.code_hash.0 != args.code_hash ||
			actual.min_item_gas != args.min_item_gas ||
			actual.min_memo_gas != args.min_memo_gas ||
			actual.parent_service != MOCK_SERVICE_ID ||
			actual.creation_slot != slot ||
			actual.last_accumulation_slot != 0 ||
			actual.deposit_offset != 0 ||
			actual.items != 2 ||
			actual.bytes != 81 + u64::from(args.len.0) ||
			storage
				.lookup_request(id, args.code_hash, args.len.0)
				.is_none_or(|r| !r.0.is_empty()) ||
			storage.lookup(id, args.code_hash).is_some()
		{
			return Err(format!("frame {frame}: created account {id} differs: {actual:?}"));
		}
		let foreign = map_entries(field(current, "foreignServices")?)?
			.into_iter()
			.find(|(key, _)| service_id(key).ok() == Some(id))
			.ok_or("created service absent from model")?
			.1;
		let requests = map_entries(field(foreign, "requests")?)?;
		if requests.len() != 1 ||
			!map_entries(field(foreign, "storage")?)?.is_empty() ||
			bounded_integer::<u32>(field(foreign, "created")?, "foreign creation slot")? != slot
		{
			return Err("created foreign request/storage metadata differs".into());
		}
		let key = tuple(requests[0].0)?;
		if key.len() != 2 ||
			variant(requests[0].1)?.0 != "Unprovided" ||
			bounded_integer::<u32>(&key[1], "foreign request length")? != args.len.0 ||
			codex.hash(integer(field(&key[0], "hashBytes")?)?, args.len.0)? != args.code_hash
		{
			return Err("created foreign request differs".into());
		}
	}
	// Seeded foreign accounts start empty and refusal-only service operations
	// must keep them empty, on both the model and the host.
	for (id, foreign) in map_entries(field(current, "foreignServices")?)? {
		let id = service_id(id)?;
		if service_id(field(foreign, "supervisor")?)? != id {
			return Err(format!("frame {frame}: foreign service {id} supervisor changed"));
		}
		if known.contains_key(&id) {
			continue;
		}
		let actual = storage.service(id).ok_or("missing foreign service")?;
		if actual.items != 0 ||
			actual.bytes != 0 ||
			!map_entries(field(foreign, "requests")?)?.is_empty() ||
			!map_entries(field(foreign, "storage")?)?.is_empty()
		{
			return Err(format!("frame {frame}: seeded foreign account {id} is not empty"));
		}
	}
	Ok(())
}
