//! Quint-generated Accumulate regressions for refine-error work results.
//! These exercise storing the errors, not their production by Rust Refine.

use serde_json::{json, Value};

use crate::itf::replay;

#[test]
fn too_many_validator_keys_works() {
	replay::trace(include_str!(
		"../fixtures/quint/refine_errors/too_many_validator_keys_works.itf.json"
	))
	.expect("the refine error and its trace should be stored");
}

#[test]
fn too_many_upward_messages_works() {
	replay::trace(include_str!(
		"../fixtures/quint/refine_errors/too_many_upward_messages_works.itf.json"
	))
	.expect("the refine error and its trace should be stored");
}

#[test]
fn restricted_host_function_works() {
	replay::trace(include_str!(
		"../fixtures/quint/refine_errors/restricted_host_function_works.itf.json"
	))
	.expect("the refine error and its trace should be stored");
}

#[test]
fn refine_output_too_large_works() {
	replay::trace(include_str!(
		"../fixtures/quint/refine_errors/refine_output_too_large_works.itf.json"
	))
	.expect("the refine error and its trace should be stored");
}

#[test]
fn missing_head_declaration_works() {
	replay::trace(include_str!(
		"../fixtures/quint/refine_errors/missing_head_declaration_works.itf.json"
	))
	.expect("the refine error and its trace should be stored");
}

#[test]
fn opaque_payload_and_auth_trace_boundaries_works() {
	for fixture in [
		include_str!("../fixtures/quint/refine_errors/opaque_0_auth_0_works.itf.json"),
		include_str!("../fixtures/quint/refine_errors/opaque_42_auth_7_works.itf.json"),
		include_str!("../fixtures/quint/refine_errors/opaque_63_auth_255_works.itf.json"),
		include_str!("../fixtures/quint/refine_errors/opaque_64_auth_256_works.itf.json"),
		include_str!("../fixtures/quint/refine_errors/opaque_1024_auth_300_works.itf.json"),
	] {
		replay::trace(fixture).expect("the opaque payload and bounded auth trace should be stored");
	}
}

// Corrupt only expected state to verify that the replay comparator rejects it.
fn missing_head_trace() -> Value {
	serde_json::from_str(include_str!(
		"../fixtures/quint/refine_errors/missing_head_declaration_works.itf.json"
	))
	.unwrap()
}

#[test]
fn incorrect_expected_error_errors() {
	let mut trace = missing_head_trace();
	trace["states"][1]["svc"]["parachainLog"]["#map"][0][1][0]["#tup"][1]["value"]["error"] =
		json!({"tag": "RestrictedHostFunction", "value": {"#tup": []}});
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("frame 1: svc.parachainLog[1] differs"), "{error}");
}

#[test]
fn incorrect_expected_auth_trace_errors() {
	let mut trace = missing_head_trace();
	trace["states"][1]["svc"]["parachainLog"]["#map"][0][1][0]["#tup"][1]["value"]["authTrace"] =
		json!({"#bigint": "0"});
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("frame 1: svc.parachainLog[1] differs"), "{error}");
}

#[test]
fn skipped_work_removes_log_errors() {
	let mut trace = missing_head_trace();
	trace["states"][2]["svc"]["parachainLog"] = json!({"#map": []});
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("frame 2: svc.parachainLog[1] differs"), "{error}");
}
