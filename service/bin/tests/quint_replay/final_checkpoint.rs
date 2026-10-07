//! Full-prefix Quint oracle: effects survive, invocation fails without a commitment.
use super::itf::replay;
use serde_json::{Value, json};

const RETAINED: &str =
	include_str!("../fixtures/quint/final_checkpoint/retained_effects_works.itf.json");

fn trace(key: &str, value: Value) -> Value {
	let mut trace: Value = serde_json::from_str(RETAINED).unwrap();
	trace["states"][2][key] = value;
	trace
}

#[test]
fn phase_interiors_works() {
	for fixture in [
		RETAINED,
		include_str!("../fixtures/quint/final_checkpoint/repeated_works.itf.json"),
		include_str!("../fixtures/quint/final_checkpoint/empty_works.itf.json"),
	] {
		for sample in [0, 250_000, 500_000, 750_000, 1_000_000] {
			let mut trace: Value = serde_json::from_str(fixture).unwrap();
			for state in trace["states"].as_array_mut().unwrap() {
				if replay::integer(&state["replayInterrupt"]).unwrap() ==
					state["lastStepWorkResults"].as_array().unwrap().len() as i128
				{
					state["replayGasSample"] = json!({"#bigint": sample.to_string()});
				}
			}
			replay::document_trace(&trace).unwrap_or_else(|e| panic!("sample {sample}: {e}"));
		}
	}
}

#[test]
fn host_boundaries_works() {
	for side in ["before", "after"] {
		let trace = trace("replayGasBoundary", json!(format!("{side}:read")));
		replay::document_trace(&trace).unwrap_or_else(|e| panic!("{side}:read: {e}"));
	}
}

#[test]
fn retained_effects_errors() {
	for (key, expected) in [
		("replayTransfers", "replayTransfers differs"),
		("replayCreations", "creation set differs"),
		("lastStepAssigns", "assign"),
	] {
		let mut trace = trace("replayGasSample", json!({"#bigint": "500000"}));
		assert!(!trace["states"][2][key].as_array().unwrap().is_empty());
		trace["states"][2][key] = json!([]);
		let error = replay::document_trace(&trace).unwrap_err();
		assert!(error.contains(expected), "{error}");
	}
}

#[test]
fn retained_state_errors() {
	let mut trace = trace("replayGasSample", json!({"#bigint": "500000"}));
	trace["states"][2]["svc"]["parachains"] = trace["states"][1]["svc"]["parachains"].clone();
	assert!(replay::document_trace(&trace).is_err());
}

#[test]
fn completed_invocation_errors() {
	let trace = trace("replayInvocationGas", json!({"#bigint": "5000000000"}));
	assert!(
		replay::document_trace(&trace)
			.unwrap_err()
			.contains("expected checkpoint NotEnoughGas")
	);
}

#[test]
fn wrong_checkpoint_errors() {
	let mut trace = trace("replayGasSample", json!({"#bigint": "500000"}));
	trace["states"][2]["replayInterrupt"] = json!({"#bigint": "2"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("headData differs"));
}

#[test]
fn returned_commitment_errors() {
	let mut trace = trace("replayGasSample", json!({"#bigint": "1000000"}));
	assert_eq!(trace["states"][1]["lastHeadRoot"]["tag"], "Some");
	trace["states"][2]["lastHeadRoot"] = trace["states"][1]["lastHeadRoot"].clone();
	assert!(replay::document_trace(&trace).unwrap_err().contains("lastHeadRoot differs"));
}
