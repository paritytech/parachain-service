//! Lookup-anchor pruning removes older entries and preserves the boundary.

use crate::itf::replay;
use serde_json::Value;

const REFINE_TRACE: &str =
	include_str!("../fixtures/quint/log_pruning/refine_log_boundary_works.itf.json");

#[test]
fn refine_log_boundary_works() {
	replay::trace(REFINE_TRACE).expect("Refine logs should prune strictly below the anchor");
}

#[test]
fn refine_log_retains_older_entry_errors() {
	wrong_pruning_errors(REFINE_TRACE, false);
}

#[test]
fn refine_log_removes_boundary_entry_errors() {
	wrong_pruning_errors(REFINE_TRACE, true);
}

fn wrong_pruning_errors(input: &str, remove_boundary: bool) {
	let mut trace: Value = serde_json::from_str(input).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.rev()
		.find(|&i| states[i]["now"] != states[i - 1]["now"])
		.expect("final candidate block");
	let previous_log = states[frame - 1]["svc"]["parachainLog"].clone();
	let log = &mut states[frame]["svc"]["parachainLog"];
	if remove_boundary {
		assert_eq!(log["#map"][0][1].as_array().unwrap().len(), 1);
		log["#map"][0][1] = serde_json::json!([]);
	} else {
		assert_eq!(previous_log["#map"][0][1].as_array().unwrap().len(), 2);
		*log = previous_log;
	}
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.starts_with(&format!("frame {frame}: svc.parachainLog[1] differs;")), "{error}");
}
