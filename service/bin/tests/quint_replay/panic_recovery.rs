use super::itf::replay;
use serde_json::{json, Value};

#[test]
fn first_checkpoint_works() {
	replay::trace(include_str!("../fixtures/quint/panic_recovery/first_checkpoint_works.itf.json"))
		.unwrap();
}

#[test]
fn middle_checkpoint_works() {
	replay::trace(include_str!(
		"../fixtures/quint/panic_recovery/middle_checkpoint_works.itf.json"
	))
	.unwrap();
}

#[test]
fn last_checkpoint_works() {
	replay::trace(include_str!("../fixtures/quint/panic_recovery/last_checkpoint_works.itf.json"))
		.unwrap();
}

#[test]
fn zero_report_gas_works() {
	replay::trace(include_str!("../fixtures/quint/panic_recovery/zero_report_gas_works.itf.json"))
		.unwrap();
}

#[test]
fn repeated_panic_works() {
	replay::trace(include_str!("../fixtures/quint/panic_recovery/repeated_panic_works.itf.json"))
		.unwrap();
}

#[test]
fn assignment_checkpoint_works() {
	replay::trace(include_str!(
		"../fixtures/quint/panic_recovery/assignment_checkpoint_works.itf.json"
	))
	.unwrap();
}

const MIDDLE: &str =
	include_str!("../fixtures/quint/panic_recovery/middle_checkpoint_works.itf.json");

#[test]
fn missing_fault_errors() {
	let mut trace: Value = serde_json::from_str(MIDDLE).unwrap();
	trace["states"][1]["replayPanic"] = json!(false);
	assert!(replay::document_trace(&trace)
		.unwrap_err()
		.contains("expected checkpoint NotEnoughGas"));
}

#[test]
fn missing_interrupt_errors() {
	let mut trace: Value = serde_json::from_str(MIDDLE).unwrap();
	trace["states"][1]["replayInterrupt"] = json!({"#bigint":"-1"});
	assert!(replay::document_trace(&trace)
		.unwrap_err()
		.contains("replayPanic requires replayInterrupt"));
}

#[test]
fn wrong_checkpoint_errors() {
	let mut trace: Value = serde_json::from_str(MIDDLE).unwrap();
	trace["states"][1]["replayInterrupt"] = json!({"#bigint":"0"});
	let error = replay::document_trace(&trace).unwrap_err();
	assert!(error.contains("headData differs"), "{error}");
}

#[test]
fn work_error_fault_errors() {
	let mut trace: Value = serde_json::from_str(MIDDLE).unwrap();
	trace["states"][1]["lastStepWorkResults"][1]["result"] =
		json!({"tag":"WorkErr", "value":{"#tup":[]}});
	assert!(replay::document_trace(&trace)
		.unwrap_err()
		.contains("panic report must have WorkOk output"));
}
