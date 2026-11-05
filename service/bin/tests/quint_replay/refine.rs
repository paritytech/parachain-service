//! Refine execution, including negative checks on the differential oracle.
use crate::itf::replay;
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("../fixtures/quint/refine_fuzz/success_works.itf.json");

#[test]
fn trivial_works() {
	for fixture in [
		SUCCESS,
		include_str!("../fixtures/quint/refine_fuzz/structured_pvf_works.itf.json"),
		include_str!("../fixtures/quint/refine_fuzz/gas_exhaustion_works.itf.json"),
		include_str!("../fixtures/quint/refine_fuzz/malformed_candidate_works.itf.json"),
		include_str!("../fixtures/quint/refine_fuzz/invalid_pvf_works.itf.json"),
		include_str!("../fixtures/quint/refine_fuzz/ordering_works.itf.json"),
		include_str!("../fixtures/quint/refine_fuzz/lookup_works.itf.json"),
	] {
		replay::trace(fixture).expect("real Refine must agree with Quint");
	}
}

#[test]
fn incorrect_digest_errors() {
	let mut trace: Value = serde_json::from_str(SUCCESS).unwrap();
	trace["states"][1]["replayRefineExpected"][0]["value"]["value"]["headData"] =
		json!({"#bigint":"9"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("digest differs"));
}

#[test]
fn incorrect_work_failure_errors() {
	let mut trace: Value = serde_json::from_str(SUCCESS).unwrap();
	trace["states"][1]["replayRefineExpected"][0] = json!({"tag":"WorkErr","value":{"#tup":[]}});
	assert!(replay::document_trace(&trace).unwrap_err().contains("expected WorkErr"));
}

#[test]
fn missing_input_errors() {
	let mut trace: Value = serde_json::from_str(SUCCESS).unwrap();
	trace["states"][1].as_object_mut().unwrap().remove("replayRefine");
	assert!(replay::document_trace(&trace)
		.unwrap_err()
		.contains("missing field replayRefine"));
}

#[test]
fn bypass_byte_fault_errors() {
	for fixture in [
		include_str!("../fixtures/quint/refine_fuzz/malformed_candidate_works.itf.json"),
		include_str!("../fixtures/quint/refine_fuzz/invalid_pvf_works.itf.json"),
	] {
		let mut trace: Value = serde_json::from_str(fixture).unwrap();
		trace["states"][1]["replayRefine"]["value"]["byteFault"] = json!("valid");
		assert!(replay::document_trace(&trace).unwrap_err().contains("expected WorkErr"));
	}
}

#[test]
fn bypass_gas_exhaustion_errors() {
	for frame in [1, 3] {
		let mut trace: Value = serde_json::from_str(include_str!(
			"../fixtures/quint/refine_fuzz/gas_exhaustion_works.itf.json"
		))
		.unwrap();
		trace["states"][frame]["replayRefine"]["value"]["gasMode"] = json!("funded");
		assert!(replay::document_trace(&trace).unwrap_err().contains("expected WorkErr"));
	}
}
