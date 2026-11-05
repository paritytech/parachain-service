//! Admission, exact author trace propagation, and downstream state from Quint.
use crate::itf::replay;
use serde_json::{json, Value};

const ACCEPTED: &str = include_str!("../fixtures/quint/authorization/accepted_works.itf.json");
const REJECTED: &str = include_str!("../fixtures/quint/authorization/rejected_works.itf.json");
const MALFORMED: &str =
	include_str!("../fixtures/quint/authorization/malformed_trace_works.itf.json");

#[test]
fn pipeline_works() {
	for fixture in [ACCEPTED, REJECTED, MALFORMED] {
		replay::trace(fixture)
			.expect("authorization admission, Refine and Accumulate must match Quint");
	}
}

#[test]
fn changed_admission_errors() {
	let mut trace: Value = serde_json::from_str(REJECTED).unwrap();
	trace["states"][1]["replayAuthorizationInputs"][0]["accepted"] = json!(true);
	assert!(replay::document_trace(&trace).unwrap_err().contains("admission differs"));
}

#[test]
fn bypass_authorization_errors() {
	let mut trace: Value = serde_json::from_str(ACCEPTED).unwrap();
	for state in trace["states"].as_array_mut().unwrap() {
		state.as_object_mut().unwrap().remove("replayAuthorizationInputs");
	}
	assert!(replay::document_trace(&trace)
		.unwrap_err()
		.contains("requires authorization inputs"));
}

#[test]
fn dropped_package_errors() {
	let mut trace: Value = serde_json::from_str(ACCEPTED).unwrap();
	trace["states"][1]["replayAuthorizationInputs"] = json!([]);
	assert!(replay::document_trace(&trace).unwrap_err().contains("count differs"));
}

#[test]
fn changed_trace_errors() {
	let mut trace: Value = serde_json::from_str(ACCEPTED).unwrap();
	trace["states"][1]["lastStepWorkResults"][0]["authTrace"] = json!({"#bigint":"32"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("report trace differs"));
}

#[test]
fn bypass_trace_validation_errors() {
	let mut trace: Value = serde_json::from_str(MALFORMED).unwrap();
	trace["states"][1]["replayAuthorizationInputs"][0]["fault"] = json!("valid");
	assert!(replay::document_trace(&trace).unwrap_err().contains("refine digest differs"));
}
