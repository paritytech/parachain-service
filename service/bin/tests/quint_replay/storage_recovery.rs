//! Storage backstop failures combined with independently selected checkpoints.
use super::itf::replay;
use serde_json::{json, Value};

const FIXTURES: [&str; 14] = [
	include_str!("../fixtures/quint/storage_recovery/queue_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/metadata_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/staging_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/balance_u30_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/balance_u32_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/balance_u40_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/balance_u48_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/balance_u56_recovery_works.itf.json"),
	include_str!(
		"../fixtures/quint/storage_recovery/validator_finalization_recovery_works.itf.json"
	),
	include_str!("../fixtures/quint/storage_recovery/assignment_payload_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/assignment_index_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/inline_assignment_recovery_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/assignment_rejected_then_due_works.itf.json"),
	include_str!("../fixtures/quint/storage_recovery/inline_uncached_tiling_works.itf.json"),
];

#[test]
fn sampled_recovery_works() {
	for (index, fixture) in FIXTURES.into_iter().enumerate() {
		for sample in [0, 250000, 500000, 750000, 1000000] {
			let mut trace: Value = serde_json::from_str(fixture).unwrap();
			for state in trace["states"].as_array_mut().unwrap().iter_mut().skip(1) {
				if state["replayInterrupt"] != json!({"#bigint":"-1"}) {
					state["replayGasSample"] = json!({"#bigint":sample.to_string()});
				}
			}
			replay::document_trace(&trace)
				.unwrap_or_else(|error| panic!("fixture {index}, sample {sample}: {error}"));
		}
	}
}

#[test]
fn lost_checkpoint_state_errors() {
	let mut trace: Value = serde_json::from_str(FIXTURES[2]).unwrap();
	trace["states"][3]["svc"]["stagedValidatorKeys"] = json!([]);
	assert!(replay::document_trace(&trace).is_err());
}

#[test]
fn lost_balance_checkpoint_errors() {
	for fixture in &FIXTURES[3..8] {
		let mut trace: Value = serde_json::from_str(fixture).unwrap();
		// Frame six retains the successful one-byte growth at the final checkpoint.
		assert_ne!(trace["states"][6]["svc"], trace["states"][5]["svc"]);
		trace["states"][6]["svc"] = trace["states"][5]["svc"].clone();
		assert!(replay::document_trace(&trace).is_err());
	}
}

#[test]
fn lost_finalization_checkpoint_errors() {
	let original: Value = serde_json::from_str(FIXTURES[8]).unwrap();
	// Rollback retains the staged buffer; the final checkpoint retains designation.
	assert_eq!(original["states"][4]["svc"]["stagedValidatorKeys"].as_array().unwrap().len(), 5);
	assert_eq!(original["states"][5]["replayDesignate"].as_array().unwrap().len(), 6);
	for field in ["replayDesignate", "replayStorageFree"] {
		let mut trace = original.clone();
		trace["states"][5][field] = trace["states"][4][field].clone();
		assert!(replay::document_trace(&trace).is_err(), "{field}");
	}
}

#[test]
fn lost_assignment_checkpoint_errors() {
	for (fixture, frame, field) in [
		(9, 6, "pendingAssigns"),
		(9, 6, "pendingAssignCores"),
		(10, 5, "pendingAssigns"),
		(10, 5, "pendingAssignCores"),
	] {
		let mut trace: Value = serde_json::from_str(FIXTURES[fixture]).unwrap();
		assert_ne!(trace["states"][frame]["svc"][field], trace["states"][frame - 1]["svc"][field]);
		trace["states"][frame]["svc"][field] = trace["states"][frame - 1]["svc"][field].clone();
		assert!(replay::document_trace(&trace).is_err(), "{fixture}/{field}");
	}
}

#[test]
fn lost_inline_call_or_cache_errors() {
	let original: Value = serde_json::from_str(FIXTURES[11]).unwrap();
	// Payload and index refusals retain the JAM call, but no partial cache.
	for frame in [3, 4, 7] {
		let mut trace = original.clone();
		assert_eq!(trace["states"][frame]["lastStepAssigns"].as_array().unwrap().len(), 1);
		trace["states"][frame]["lastStepAssigns"] = json!([]);
		assert!(replay::document_trace(&trace).is_err(), "lost call at {frame}");
	}
	// The failed larger inline queue must preserve the previous cached queue.
	let mut trace = original;
	trace["states"][7]["svc"]["pendingAssigns"] =
		trace["states"][8]["svc"]["pendingAssigns"].clone();
	assert!(replay::document_trace(&trace).is_err());
}

#[test]
fn lost_due_flush_errors() {
	let original: Value = serde_json::from_str(FIXTURES[12]).unwrap();
	assert_eq!(original["states"][4]["lastStepAssigns"].as_array().unwrap().len(), 1);
	for field in ["pendingAssigns", "pendingAssignCores"] {
		let mut trace = original.clone();
		trace["states"][4]["svc"][field] = trace["states"][3]["svc"][field].clone();
		assert!(replay::document_trace(&trace).is_err(), "{field}");
	}
}
