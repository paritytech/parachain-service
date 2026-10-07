//! Quint predicts full guest rollback, retaining only JAM's incoming credits.
use super::itf::replay;
use serde_json::{json, Value};

const DUE: &str = include_str!("../fixtures/quint/pre_checkpoint/due_assignment_works.itf.json");

#[test]
fn phase_interiors_works() {
	for fixture in [
		DUE,
		include_str!("../fixtures/quint/pre_checkpoint/zero_credit_works.itf.json"),
		include_str!("../fixtures/quint/pre_checkpoint/repeated_works.itf.json"),
		include_str!("../fixtures/quint/pre_checkpoint/empty_works.itf.json"),
	] {
		for sample in [0, 250_000, 500_000, 750_000, 1_000_000] {
			let mut trace: Value = serde_json::from_str(fixture).unwrap();
			for state in trace["states"].as_array_mut().unwrap() {
				if state["replayInterrupt"]["#bigint"] == "-2" {
					state["replayGasSample"] = json!({"#bigint": sample.to_string()});
				}
			}
			replay::document_trace(&trace).unwrap_or_else(|e| panic!("sample {sample}: {e}"));
		}
	}
}

#[test]
fn host_boundaries_works() {
	for (fixture, frame, call) in [
		(DUE, 2, "assign"),
		(include_str!("../fixtures/quint/pre_checkpoint/incoming_works.itf.json"), 1, "write"),
	] {
		for side in ["before", "after"] {
			let mut trace: Value = serde_json::from_str(fixture).unwrap();
			trace["states"][frame]["replayGasBoundary"] = json!(format!("{side}:{call}"));
			replay::document_trace(&trace).unwrap();
		}
	}
}

#[test]
fn wrong_phase_errors() {
	let mut trace: Value = serde_json::from_str(DUE).unwrap();
	trace["states"][2]["replayInterrupt"] = json!({"#bigint": "0"});
	trace["states"][2]["replayGasSample"] = json!({"#bigint": "500000"});
	assert!(replay::document_trace(&trace).is_err());
}

#[test]
fn lost_credit_errors() {
	let mut trace: Value = serde_json::from_str(DUE).unwrap();
	trace["states"][2]["svc"]["jamAccount"] = trace["states"][1]["svc"]["jamAccount"].clone();
	assert!(replay::document_trace(&trace).unwrap_err().contains("balance"));
}

#[test]
fn retained_queue_errors() {
	let mut trace: Value = serde_json::from_str(DUE).unwrap();
	trace["states"][2]["svc"]["incomingTransferBuckets"] =
		trace["states"][3]["svc"]["incomingTransferBuckets"].clone();
	assert!(replay::document_trace(&trace).unwrap_err().contains("incomingTransferBuckets"));
}

#[test]
fn retained_assignment_errors() {
	let mut trace: Value = serde_json::from_str(DUE).unwrap();
	trace["states"][2]["lastStepAssigns"] = trace["states"][3]["lastStepAssigns"].clone();
	assert!(!trace["states"][2]["lastStepAssigns"].as_array().unwrap().is_empty());
	assert!(replay::document_trace(&trace).unwrap_err().contains("assign"));
}

#[test]
fn completed_checkpoint_errors() {
	let mut trace: Value = serde_json::from_str(DUE).unwrap();
	trace["states"][2]["replayInvocationGas"] = json!({"#bigint": "5000000000"});
	assert!(replay::document_trace(&trace)
		.unwrap_err()
		.contains("expected checkpoint NotEnoughGas"));
}
