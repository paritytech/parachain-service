//! Original refine inputs and both stages' independent model expectations.
use crate::itf::replay;
use serde_json::{json, Value};

const SINGLE: &str = include_str!("../fixtures/quint/combined/single_works.itf.json");

#[test]
fn trivial_works() {
	for trace in [
		SINGLE,
		include_str!("../fixtures/quint/combined/gas_cutoffs_works.itf.json"),
		include_str!("../fixtures/quint/combined/malformed_works.itf.json"),
		include_str!("../fixtures/quint/combined/ordered_batch_works.itf.json"),
		include_str!("../fixtures/quint/combined/errors_works.itf.json"),
		include_str!("../fixtures/quint/combined/checkpoint_works.itf.json"),
	] {
		replay::trace(trace).expect("Refine outputs and Accumulate state must match Quint");
	}
}

#[test]
fn changed_script_errors() {
	let mut trace: Value = serde_json::from_str(SINGLE).unwrap();
	trace["states"][1]["replayRefineInputs"]["value"][0]["calls"][1]["value"] =
		json!({"#bigint":"9"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("refine: digest differs"));
}

#[test]
fn changed_digest_errors() {
	let mut trace: Value = serde_json::from_str(SINGLE).unwrap();
	trace["states"][1]["lastStepWorkResults"][0]["result"]["value"]["value"]["headData"] =
		json!({"#bigint":"9"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("refine: digest differs"));
}

#[test]
fn changed_accumulate_state_errors() {
	let mut trace: Value = serde_json::from_str(SINGLE).unwrap();
	trace["states"][1]["svc"]["parachains"]["#map"][0][1]["headData"] = json!({"#bigint":"9"});
	let error = replay::document_trace(&trace).unwrap_err();
	assert!(error.contains("headData"), "{error}");
}

#[test]
fn dropped_input_errors() {
	let mut trace: Value = serde_json::from_str(SINGLE).unwrap();
	trace["states"][1]["replayRefineInputs"]["value"] = json!([]);
	assert!(replay::document_trace(&trace).unwrap_err().contains("count differs"));
}

#[test]
fn bypass_refine_errors() {
	let mut trace: Value = serde_json::from_str(SINGLE).unwrap();
	trace["states"][1]["replayRefineInputs"] = json!({"tag":"None","value":{"#tup":[]}});
	assert!(replay::document_trace(&trace).unwrap_err().contains("requires refine inputs"));
}

#[test]
fn bypass_byte_fault_errors() {
	for (frame, count) in [(1, 3), (2, 5)] {
		for input in 0..count {
			let mut trace: Value = serde_json::from_str(include_str!(
				"../fixtures/quint/combined/malformed_works.itf.json"
			))
			.unwrap();
			trace["states"][frame]["replayRefineInputs"]["value"][input]["byteFault"] =
				json!("valid");
			assert!(replay::document_trace(&trace).unwrap_err().contains("expected WorkErr"));
		}
	}
}

#[test]
fn bypass_gas_fault_errors() {
	for frame in [1, 2, 3, 4, 5] {
		let mut trace: Value = serde_json::from_str(include_str!(
			"../fixtures/quint/combined/gas_cutoffs_works.itf.json"
		))
		.unwrap();
		trace["states"][frame]["replayRefineInputs"]["value"][0]["gasMode"] = json!("funded");
		assert!(replay::document_trace(&trace).unwrap_err().contains("expected WorkErr"));
	}
}

#[test]
fn invalid_gas_sample_errors() {
	let mut trace: Value =
		serde_json::from_str(include_str!("../fixtures/quint/combined/gas_cutoffs_works.itf.json"))
			.unwrap();
	trace["states"][3]["replayRefineInputs"]["value"][0]["gasSample"] =
		json!({"#bigint":"1000001"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("gas sample exceeds"));
}
