//! Hand-authored ITF regression scenarios for Accumulate's refine-error path.
//! These specialize the existing fixture; they are not Quint-generated traces
//! and do not exercise production of the error by Rust Refine.

use serde_json::{json, Value};

use crate::common::itf::replay;

fn scenario(error: Value, auth_len: u32, stored_len: u32) -> Value {
	let mut trace: Value =
		serde_json::from_str(include_str!("../fixtures/quint/refine_error_replay.itf.json"))
			.unwrap();
	let work = &mut trace["states"][1]["lastStepWorkResults"][0];
	work["result"]["value"]["value"]["error"] = error.clone();
	work["authTrace"] = json!({"#bigint": auth_len.to_string()});
	// Frame 1 appends the error; frame 2's JAM work error must preserve it.
	for frame in [1, 2] {
		let entry =
			&mut trace["states"][frame]["svc"]["parachainLog"]["#map"][0][1][0]["#tup"][1]["value"];
		entry["error"] = error.clone();
		entry["authTrace"] = json!({"#bigint": stored_len.to_string()});
	}
	trace
}

fn nullary(tag: &str) -> Value {
	json!({"tag": tag, "value": {"#tup": []}})
}

fn replay_nullary(tag: &str) {
	let trace = scenario(nullary(tag), 7, 7);
	replay::trace(&trace.to_string()).expect("the refine error and its trace should be stored");
}

#[test]
fn too_many_validator_keys_works() {
	replay_nullary("SetValidatorKeysTooManyKeys");
}

#[test]
fn too_many_upward_messages_works() {
	replay_nullary("TooManyUpwardMessages");
}

#[test]
fn restricted_host_function_works() {
	replay_nullary("RestrictedHostFunction");
}

#[test]
fn refine_output_too_large_works() {
	replay_nullary("RefineOutputTooLarge");
}

#[test]
fn missing_head_declaration_works() {
	replay_nullary("MissingHeadDeclaration");
}

#[test]
fn opaque_payload_and_auth_trace_boundaries_works() {
	for (payload_len, auth_len, stored_len) in
		[(0, 0, 0), (42, 7, 7), (63, 255, 255), (64, 256, 256), (1024, 300, 256)]
	{
		let error = json!({"tag": "Opaque", "value": {"#bigint": payload_len.to_string()}});
		let trace = scenario(error, auth_len, stored_len);
		replay::trace(&trace.to_string()).unwrap_or_else(|error| {
			panic!("payload={payload_len}, auth={auth_len}, stored={stored_len}: {error}")
		});
	}
}

#[test]
fn incorrect_expected_error_errors() {
	let mut trace = scenario(nullary("RestrictedHostFunction"), 7, 7);
	trace["states"][1]["svc"]["parachainLog"]["#map"][0][1][0]["#tup"][1]["value"]["error"] =
		nullary("MissingHeadDeclaration");
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("frame 1: svc.parachainLog[1] differs"), "{error}");
}

#[test]
fn incorrect_expected_auth_trace_errors() {
	let trace = scenario(nullary("MissingHeadDeclaration"), 7, 0);
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("frame 1: svc.parachainLog[1] differs"), "{error}");
}

#[test]
fn skipped_work_removes_log_errors() {
	let mut trace = scenario(nullary("TooManyUpwardMessages"), 7, 7);
	trace["states"][2]["svc"]["parachainLog"] = json!({"#map": []});
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("frame 2: svc.parachainLog[1] differs"), "{error}");
}
