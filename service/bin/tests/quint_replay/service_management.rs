use super::itf::replay;
use serde_json::{json, Value};

const REFUSALS: &str = include_str!("../fixtures/quint/service_management/refusals_works.itf.json");
const CREATED: &str =
	include_str!("../fixtures/quint/service_management/created_account_works.itf.json");

#[test]
fn refusals_works() {
	replay::trace(REFUSALS).unwrap();
}

#[test]
fn created_account_works() {
	replay::trace(CREATED).unwrap();
}

#[test]
fn unauthorized_works() {
	replay::trace(include_str!("../fixtures/quint/service_management/unauthorized_works.itf.json"))
		.unwrap();
}

#[test]
fn stale_work_works() {
	replay::trace(include_str!("../fixtures/quint/service_management/stale_work_works.itf.json"))
		.unwrap();
}

#[test]
fn gas_rejection_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_management/gas_rejection_works.itf.json"
	))
	.unwrap();
}

#[test]
fn checkpoint_first_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_management/checkpoint_first_works.itf.json"
	))
	.unwrap();
}

#[test]
fn checkpoint_middle_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_management/checkpoint_middle_works.itf.json"
	))
	.unwrap();
}

#[test]
fn checkpoint_last_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_management/checkpoint_last_works.itf.json"
	))
	.unwrap();
}

#[test]
fn host_budget_works() {
	replay::trace(include_str!("../fixtures/quint/service_management/host_budget_works.itf.json"))
		.unwrap();
}

#[test]
fn changed_target_errors() {
	for message in 0..5 {
		let mut trace: Value = serde_json::from_str(REFUSALS).unwrap();
		trace["states"][1]["lastStepWorkResults"][0]["result"]["value"]["value"]
			["upwardMessages"][message]["value"]["service"]["value"] = json!({"#bigint":"7"});
		assert!(replay::document_trace(&trace).unwrap_err().contains("parachainLog"));
	}
}

#[test]
fn changed_supervisor_errors() {
	let mut trace: Value = serde_json::from_str(REFUSALS).unwrap();
	// Target 7 exists: changing the missing new supervisor to self must
	// change UnknownNewSupervisor to NotSupervised.
	trace["states"][2]["lastStepWorkResults"][0]["result"]["value"]["value"]["upwardMessages"][1]
		["value"]["newSupervisor"]["value"] = json!({"#bigint":"1"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("parachainLog"));
}

#[test]
fn invalid_storage_key_errors() {
	let mut trace: Value = serde_json::from_str(REFUSALS).unwrap();
	trace["states"][1]["lastStepWorkResults"][0]["result"]["value"]["value"]["upwardMessages"][0]
		["value"]["key"][0] = json!({"#bigint":"256"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("byte"));
}

#[test]
fn changed_model_supervisor_errors() {
	let mut trace: Value = serde_json::from_str(CREATED).unwrap();
	let foreign = trace["states"][1]["foreignServices"]["#map"]
		.as_array_mut()
		.unwrap()
		.iter_mut()
		.find(|entry| entry[0]["value"]["#bigint"] == "42")
		.unwrap();
	foreign[1]["supervisor"]["value"] = json!({"#bigint":"7"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("supervisor changed"));
}
