//! Lookup-anchor pruning removes older entries and preserves the boundary.

use crate::itf::replay;
use serde_json::Value;

#[test]
fn stale_parent_seed_1_works() {
	replay::trace(include_str!("../fixtures/quint/log_pruning/stale_parent_seed_1_works.itf.json"))
		.expect("rejected candidates prune logs under the pinned spec");
}

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

const ACCUMULATE_TRACE: &str =
	include_str!("../fixtures/quint/log_pruning/accumulate_log_boundary_works.itf.json");

#[test]
fn accumulate_log_boundary_works() {
	replay::trace(ACCUMULATE_TRACE)
		.expect("successful forget logs should prune strictly below the anchor");
}

#[test]
fn accumulate_log_retains_older_entry_errors() {
	wrong_pruning_errors(ACCUMULATE_TRACE, false);
}

#[test]
fn accumulate_log_removes_boundary_entry_errors() {
	wrong_pruning_errors(ACCUMULATE_TRACE, true);
}

#[test]
fn accumulate_log_wrong_deadline_errors() {
	let mut trace: Value = serde_json::from_str(ACCUMULATE_TRACE).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.find(|&i| {
			states[i]["now"] != states[i - 1]["now"] &&
				!states[i]["svc"]["parachainLog"]["#map"].as_array().unwrap().is_empty()
		})
		.expect("first successful forget block");
	states[frame]["svc"]["parachainLog"]["#map"][0][1][0]["#tup"][1]["value"][0]["value"]["due"] =
		serde_json::json!({"#bigint": "0"});
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.starts_with(&format!("frame {frame}: svc.parachainLog[1] differs;")), "{error}");
}
