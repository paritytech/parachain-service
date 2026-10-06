//! Replay-only host budget inputs and log vocabulary. Expected transitions and
//! deposit deltas are computed in Quint; this adapter never predicts a write.
use std::borrow::Cow;

use jam_node::vm::Storage;
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use serde_json::{json, Value};

use super::replay::*;

const FIELDS: [&str; 5] = [
	"replayStorageBudget",
	"replayStorageFree",
	"replayStorageLogs",
	"replayStorageSizes",
	"replayFailedHeads",
];

/// The pin has no ParaInfo/IncomingTransfer backstop log variants. Translate
/// the extension's log vocabulary into the common comparison representation.
pub fn normalize(document: &Value) -> Result<Cow<'_, Value>, String> {
	let states = field(document, "states")?.as_array().ok_or("missing states")?;
	let profile = states.first().is_some_and(|s| s.get(FIELDS[0]).is_some());
	for state in states {
		if FIELDS.iter().any(|key| state.get(key).is_some() != profile) {
			return Err("storage budget traces require every storage field in every frame".into());
		}
	}
	if !profile {
		return Ok(Cow::Borrowed(document));
	}
	if jam_types::deposit_per_item() != 10 || jam_types::deposit_per_byte() != 1 {
		return Err("storage budget profile requires JAM deposits 10/item and 1/byte".into());
	}
	let mut document = document.clone();
	for state in document["states"].as_array_mut().unwrap() {
		bounded_integer::<u64>(field(state, FIELDS[0])?, FIELDS[0])?;
		bounded_integer::<u64>(field(state, FIELDS[1])?, FIELDS[1])?;
		let reports = field(state, "lastStepWorkResults")?.as_array().ok_or("expected reports")?;
		let mut seen = std::collections::BTreeSet::new();
		for index in field(state, "replayFailedHeads")?.as_array().ok_or("expected failed heads")? {
			let index = bounded_integer::<usize>(index, "failed head index")?;
			if index >= reports.len() || !seen.insert(index) {
				return Err("invalid failed head index".into());
			}
		}
		let mut logs = Vec::new();
		for entry in field(state, "replayStorageLogs")?.as_array().ok_or("expected storage logs")? {
			let pair = tuple(entry)?;
			if pair.len() != 2 {
				return Err("invalid storage log entry".into());
			}
			let mut events = Vec::new();
			for reason in pair[1].as_array().ok_or("expected storage reasons")? {
				let (tag, payload) = variant(reason)?;
				let reason = match tag {
					"HeadWrite" => json!({"tag":"FromParaInfo", "value":{"#tup":[]}}),
					"QueueWrite" => json!({"tag":"FromIncomingTransfer", "value":{"#tup":[]}}),
					"KVWrite" => {
						let key = bounded_integer::<u8>(payload, "storage KV key")?;
						json!({"tag":"FromSetKV", "value":{"keyHash":{"kvKeyBytes":{"#bigint":key.to_string()}}}})
					},
					_ => return Err(format!("unsupported storage reason {tag}")),
				};
				events.push(json!({"tag":"InsufficientStateBalance", "value":reason}));
			}
			logs.push(json!({"#tup":[pair[0], {"tag":"AccumulateLogEntry", "value":events}]}));
		}
		state["svc"]["parachainLog"] = json!({"#map":[[
			{"tag":"MkParaId", "value":{"#bigint":"2"}}, logs
		]]});
	}
	Ok(Cow::Owned(document))
}

pub fn prepare(storage: &mut Storage, frame: &Value) -> Result<(), String> {
	let Some(budget) = frame.get("replayStorageBudget") else { return Ok(()) };
	let budget = bounded_integer::<u64>(budget, "storage budget")?;
	let mut service = storage.service(MOCK_SERVICE_ID).ok_or("missing budgeted service")?;
	// An explicit fault-environment input, relative to the current footprint.
	// Do not patch counters or intercept write calls: exercise JAM's real FULL.
	service.balance = service.threshold().checked_add(budget).ok_or("storage budget overflow")?;
	storage.set_service(MOCK_SERVICE_ID, &service);
	storage.commit();
	Ok(())
}

pub fn compare(storage: &Storage, frame: &Value, index: usize) -> Result<(), String> {
	let Some(free) = frame.get("replayStorageFree") else { return Ok(()) };
	let expected = bounded_integer::<u64>(free, "storage free balance")?;
	let service = storage.service(MOCK_SERVICE_ID).ok_or("missing budgeted service")?;
	if service.balance < service.threshold() || service.free() != expected {
		return Err(format!(
			"frame {index}: storage free balance differs: expected {expected}, actual {}",
			service.free()
		));
	}
	Ok(())
}

pub fn failed_head(frame: &Value, index: usize) -> Result<bool, String> {
	let Some(indices) = frame.get("replayFailedHeads") else { return Ok(false) };
	for value in indices.as_array().ok_or("expected failed heads")? {
		if bounded_integer::<usize>(value, "failed head index")? == index {
			return Ok(true);
		}
	}
	Ok(false)
}
