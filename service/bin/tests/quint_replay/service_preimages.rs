use super::itf::replay;
use serde_json::{json, Value};

const REFUSALS: &str = include_str!("../fixtures/quint/service_preimages/refusals_works.itf.json");
const CREATED: &str =
	include_str!("../fixtures/quint/service_preimages/created_request_works.itf.json");

#[test]
fn refusals_works() {
	replay::trace(REFUSALS).unwrap();
}

#[test]
fn created_request_works() {
	replay::trace(CREATED).unwrap();
}

#[test]
fn unauthorized_works() {
	replay::trace(include_str!("../fixtures/quint/service_preimages/unauthorized_works.itf.json"))
		.unwrap();
}

#[test]
fn stale_work_works() {
	replay::trace(include_str!("../fixtures/quint/service_preimages/stale_work_works.itf.json"))
		.unwrap();
}

#[test]
fn gas_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/service_preimages/gas_rejection_works.itf.json"))
		.unwrap();
}

#[test]
fn checkpoint_first_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_preimages/checkpoint_first_works.itf.json"
	))
	.unwrap();
}

#[test]
fn checkpoint_middle_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_preimages/checkpoint_middle_works.itf.json"
	))
	.unwrap();
}

#[test]
fn checkpoint_last_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_preimages/checkpoint_last_works.itf.json"
	))
	.unwrap();
}

#[test]
fn host_budget_works() {
	replay::trace(include_str!("../fixtures/quint/service_preimages/host_budget_works.itf.json"))
		.unwrap();
}

#[test]
fn changed_target_errors() {
	for message in 0..2 {
		let mut trace: Value = serde_json::from_str(REFUSALS).unwrap();
		// The oracle expects UnknownService; service 7 exists and must instead
		// produce NotSupervised, with no preimage effects in either case.
		trace["states"][1]["lastStepWorkResults"][0]["result"]["value"]["value"]
			["upwardMessages"][message]["value"]["target"]["value"]["value"] = json!({"#bigint":"7"});
		assert!(replay::document_trace(&trace).unwrap_err().contains("parachainLog"));
	}
}

#[test]
fn changed_refusal_log_errors() {
	let path = "/states/1/svc/parachainLog/#map/0/1/0/#tup/1/value";
	for message in 0..2 {
		for (field, value) in [
			("service", json!({"tag":"MkServiceId", "value":{"#bigint":"7"}})),
			(
				"error",
				json!({"tag": if message == 0 { "SolicitNotSupervised" } else { "StoreNotSupervised" }, "value":{"#tup":[]}}),
			),
		] {
			let mut trace: Value = serde_json::from_str(REFUSALS).unwrap();
			trace.pointer_mut(path).unwrap()[message]["value"][field] = value;
			assert!(replay::document_trace(&trace).unwrap_err().contains("parachainLog"));
		}
	}
	let mut trace: Value = serde_json::from_str(REFUSALS).unwrap();
	*trace.pointer_mut(path).unwrap() = json!([]);
	assert!(replay::document_trace(&trace).unwrap_err().contains("parachainLog"));
}

#[test]
fn removed_created_request_errors() {
	let mut trace: Value = serde_json::from_str(CREATED).unwrap();
	let foreign = trace["states"][1]["foreignServices"]["#map"]
		.as_array_mut()
		.unwrap()
		.iter_mut()
		.find(|entry| entry[0]["value"]["#bigint"] == "42")
		.unwrap();
	foreign[1]["requests"] = json!({"#map": []});
	assert!(replay::document_trace(&trace)
		.unwrap_err()
		.contains("request/storage metadata differs"));
}

#[test]
fn unexpected_seeded_request_errors() {
	let created: Value = serde_json::from_str(CREATED).unwrap();
	let requests = created["states"][1]["foreignServices"]["#map"]
		.as_array()
		.unwrap()
		.iter()
		.find(|entry| entry[0]["value"]["#bigint"] == "42")
		.unwrap()[1]["requests"]
		.clone();
	let mut trace: Value = serde_json::from_str(REFUSALS).unwrap();
	trace["states"][1]["foreignServices"]["#map"][0][1]["requests"] = requests;
	assert!(replay::document_trace(&trace).unwrap_err().contains("is not empty"));
}
