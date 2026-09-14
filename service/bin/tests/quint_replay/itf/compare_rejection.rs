//! Expected-state mutations must report the frame and field that disagree.

use serde_json::{json, Value};

use super::{codex::Codex, compare, replay, seed};
use crate::common::fresh_storage;

fn frame() -> Value {
	let trace: Value = serde_json::from_str(include_str!(
		"../../fixtures/quint/staleParentCandidateRejectedTest.itf.json"
	))
	.unwrap();
	trace["states"][0].clone()
}

fn rejects_change(mut expected: Value, pointer: &str, replacement: Value, field: &str) {
	let mut codex = Codex::default();
	let storage = fresh_storage(|storage| seed::seed(storage, &expected, &mut codex).unwrap());
	compare::state(&storage, &expected, &mut codex, 17).expect("unmodified state should match");
	let value = expected.pointer_mut(pointer).expect("expected field exists");
	assert_ne!(*value, replacement, "mutation must change {pointer}");
	*value = replacement;
	let error = compare::state(&storage, &expected, &mut codex, 17).unwrap_err();
	assert!(error.starts_with(&format!("frame 17: svc.parachains[1].{field} differs;")), "{error}");
}

fn rejects_para_change(pointer: &str, replacement: Value, field: &str) {
	rejects_change(frame(), &format!("/svc/parachains/#map/0/1/{pointer}"), replacement, field);
}

#[test]
fn head_data_errors() {
	rejects_para_change("headData/#bigint", json!("9"), "headData");
}

#[test]
fn validation_code_hash_errors() {
	rejects_para_change(
		"validationCode/value/ref/hash/vchBytes/#bigint",
		json!("2"),
		"validationCode",
	);
}

#[test]
fn validation_code_presence_errors() {
	rejects_para_change(
		"validationCode",
		json!({"tag": "None", "value": {"#tup": []}}),
		"validationCode",
	);
}

#[test]
fn validation_code_pinned_errors() {
	rejects_para_change("validationCode/value/pinned", json!(true), "validationCode");
}

#[test]
fn total_state_balance_errors() {
	rejects_para_change("totalStateBalance/#bigint", json!("271141"), "totalStateBalance");
}

#[test]
fn used_state_balance_errors() {
	rejects_para_change("usedStateBalance/#bigint", json!("135571"), "usedStateBalance");
}

#[test]
fn deregistering_errors() {
	rejects_para_change("isDeregistering", json!(true), "isDeregistering");
}

#[test]
fn pending_upgrade_presence_errors() {
	let code = frame()["svc"]["parachains"]["#map"][0][1]["validationCode"]["value"].clone();
	rejects_para_change(
		"pendingUpgrade",
		json!({"tag": "Some", "value": {"#tup": [code, {"#bigint": "7"}]}}),
		"pendingUpgrade",
	);
}

#[test]
fn parachain_log_order_errors() {
	let mut trace: Value = serde_json::from_str(include_str!(
		"../../fixtures/quint/refine_errors/mixed_errors_preserve_log_works.itf.json"
	))
	.unwrap();
	replay::trace(&trace.to_string()).expect("unmodified trace should match");
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.find(|&i| {
			states[i]["now"] != states[i - 1]["now"] &&
				states[i]["svc"]["parachainLog"]["#map"][0][1]
					.as_array()
					.is_some_and(|entries| entries.len() >= 2)
		})
		.expect("block with at least two log entries");
	let entries = states[frame]["svc"]["parachainLog"]["#map"][0][1].as_array_mut().unwrap();
	assert_ne!(entries[0], entries[1]);
	entries.swap(0, 1);
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains(&format!("frame {frame}: svc.parachainLog[1] differs;")), "{error}");
}
