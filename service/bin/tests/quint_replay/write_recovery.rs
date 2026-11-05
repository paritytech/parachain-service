//! Quint fixes the committed prefix before gas calibration locates each write.
use super::itf::replay;
use serde_json::{json, Value};

const FIXTURES: [(&str, &str, usize); 5] = [
	(
		"cleanup_partial",
		include_str!("../fixtures/quint/host_mixed/cleanup_partial_recovery_works.itf.json"),
		4,
	),
	(
		"cleanup_full",
		include_str!("../fixtures/quint/host_mixed/cleanup_full_recovery_works.itf.json"),
		4,
	),
	(
		"remove_refund",
		include_str!("../fixtures/quint/host_mixed/remove_refund_recovery_works.itf.json"),
		3,
	),
	(
		"forget_refund",
		include_str!("../fixtures/quint/host_mixed/forget_refund_recovery_works.itf.json"),
		3,
	),
	(
		"code_apply",
		include_str!("../fixtures/quint/host_mixed/code_apply_recovery_works.itf.json"),
		2,
	),
];

#[test]
fn write_boundaries_works() {
	for (name, fixture, minimum_writes) in FIXTURES {
		let original: Value = serde_json::from_str(fixture).unwrap();
		let index = original["states"]
			.as_array()
			.unwrap()
			.iter()
			.position(|s| s["replayInterrupt"]["#bigint"] == "0")
			.unwrap();
		let mut writes = 0;
		for occurrence in 0..64 {
			let mut trace = original.clone();
			trace["states"][index]["replayGasBoundary"] =
				json!(format!("before:write:{occurrence}"));
			match replay::document_trace(&trace) {
				Err(error) if error.contains("gas boundary call absent from selected phase") => {
					break
				},
				result => {
					result.unwrap_or_else(|e| panic!("{name}, before write {occurrence}: {e}"))
				},
			}
			trace["states"][index]["replayGasBoundary"] =
				json!(format!("after:write:{occurrence}"));
			replay::document_trace(&trace)
				.unwrap_or_else(|e| panic!("{name}, after write {occurrence}: {e}"));
			writes += 1;
		}
		assert!((minimum_writes..64).contains(&writes), "{name}: {writes} writes");
	}
}

#[test]
fn checkpoint_interiors_works() {
	for (name, fixture, _) in FIXTURES {
		for sample in [0, 250000, 500000, 750000, 1000000] {
			let mut trace: Value = serde_json::from_str(fixture).unwrap();
			for state in trace["states"].as_array_mut().unwrap().iter_mut().skip(1) {
				if matches!(state["replayInterrupt"]["#bigint"].as_str(), Some("0" | "1")) {
					state["replayGasSample"] = json!({"#bigint": sample.to_string()});
				}
			}
			replay::document_trace(&trace).unwrap_or_else(|e| panic!("{name}, {sample}: {e}"));
		}
	}
}

#[test]
fn wrong_checkpoint_errors() {
	for (name, fixture, _) in FIXTURES {
		let mut trace: Value = serde_json::from_str(fixture).unwrap();
		let state = trace["states"]
			.as_array_mut()
			.unwrap()
			.iter_mut()
			.find(|s| s["replayInterrupt"]["#bigint"] == "0")
			.unwrap();
		// Commit the report while retaining Quint's rollback expectation.
		state["replayInterrupt"] = json!({"#bigint": "1"});
		state["replayGasSample"] = json!({"#bigint": "500000"});
		let error = replay::document_trace(&trace).unwrap_err();
		assert!(error.contains("differs"), "{name}: {error}");
	}
}
