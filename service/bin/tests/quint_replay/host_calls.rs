//! Intermediate effects must match even when final host state is unchanged.
use super::itf::replay;
use serde_json::{json, Value};

const ORDERED: &str = include_str!("../fixtures/quint/host_calls/ordered_works.itf.json");
const RECOVERY: [&str; 5] = [
	include_str!("../fixtures/quint/host_calls/pre_checkpoint_works.itf.json"),
	include_str!("../fixtures/quint/host_calls/first_checkpoint_works.itf.json"),
	include_str!("../fixtures/quint/host_calls/middle_checkpoint_works.itf.json"),
	include_str!("../fixtures/quint/host_calls/last_checkpoint_works.itf.json"),
	include_str!("../fixtures/quint/host_calls/final_checkpoint_works.itf.json"),
];

#[test]
fn ordered_and_repeated_works() {
	replay::trace(ORDERED).unwrap();
}

#[test]
fn checkpoint_recovery_works() {
	for fixture in RECOVERY {
		replay::trace(fixture).unwrap();
		for sample in [0, 250000, 500000, 750000, 1000000] {
			let mut trace: Value = serde_json::from_str(fixture).unwrap();
			for state in trace["states"].as_array_mut().unwrap().iter_mut().skip(1) {
				if state["replayInterrupt"]["#bigint"] != "-1" {
					state["replayPanic"] = json!(false);
					state["replayGasSample"] = json!({"#bigint":sample.to_string()});
				}
			}
			replay::document_trace(&trace).unwrap();
		}
	}
}

#[test]
fn overwritten_and_repeated_calls_errors() {
	let original: Value = serde_json::from_str(ORDERED).unwrap();
	let index = original["states"]
		.as_array()
		.unwrap()
		.iter()
		.position(|s| s["replayHostCalls"].as_array().unwrap().len() == 9)
		.unwrap();
	for mutation in 0..6 {
		let mut trace = original.clone();
		let calls = trace["states"][index]["replayHostCalls"].as_array_mut().unwrap();
		match mutation {
			0 => {
				calls.remove(0);
			},
			1 => calls.swap(0, 1),
			2 => calls[0]["value"]["codeHash"]["hashBytes"] = json!({"#bigint":"9002"}),
			3 => calls[0]["value"]["minAccGas"] = json!({"#bigint":"34"}),
			4 => calls[1]["value"][0] = json!({"#bigint":"99"}),
			_ => {
				calls.pop();
			},
		}
		let error = replay::document_trace(&trace).unwrap_err();
		assert!(error.contains("ordered replayHostCalls differ"), "{error}");
	}
}

#[test]
fn missing_history_errors() {
	for remove_all in [false, true] {
		let mut trace: Value = serde_json::from_str(ORDERED).unwrap();
		for (index, state) in trace["states"].as_array_mut().unwrap().iter_mut().enumerate() {
			if remove_all || index == 1 {
				state.as_object_mut().unwrap().remove("replayHostCalls");
			}
		}
		assert!(replay::document_trace(&trace).unwrap_err().contains("replayHostCalls"));
	}
}
