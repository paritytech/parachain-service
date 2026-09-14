//! Replay empty blocks, skipped work, and multiple work packages in one block.

use crate::itf::replay;

#[test]
fn ump_ordering_works() {
	replay::trace(include_str!("../fixtures/quint/blocks/ump_ordering_works.itf.json")).expect(
		"UMP ordering, shared references, and refunds should match Quint across WPs and blocks",
	);
}

#[test]
fn two_work_packages_works() {
	replay::trace(include_str!("../fixtures/quint/blocks/two_work_packages_works.itf.json"))
		.expect("both parachain heads and their combined commitment should match Quint");
}

#[test]
fn work_error_between_candidates_works() {
	replay::trace(include_str!(
		"../fixtures/quint/blocks/work_error_between_candidates_works.itf.json"
	))
	.expect("skipped work should preserve the head used by the next candidate");
}

#[test]
fn empty_blocks_between_errors_works() {
	replay::trace(include_str!(
		"../fixtures/quint/blocks/empty_blocks_between_errors_works.itf.json"
	))
	.expect("Quint and Rust should agree after work and empty blocks");
}

#[test]
fn empty_blocks_between_candidates_works() {
	replay::trace(include_str!(
		"../fixtures/quint/blocks/empty_blocks_between_candidates_works.itf.json"
	))
	.expect("Quint and Rust should agree after work and empty blocks");
}

#[test]
fn empty_blocks_between_error_and_candidate_works() {
	replay::trace(include_str!(
		"../fixtures/quint/blocks/empty_blocks_between_error_and_candidate_works.itf.json"
	))
	.expect("Quint and Rust should agree after work and empty blocks");
}

#[test]
fn empty_block_removes_log_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/blocks/empty_blocks_between_errors_works.itf.json"
	))
	.unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.find(|&i| {
			states[i]["now"] != states[i - 1]["now"] &&
				states[i]["lastStepWorkResults"].as_array().unwrap().is_empty() &&
				!states[i]["svc"]["parachainLog"]["#map"].as_array().unwrap().is_empty()
		})
		.expect("empty block after a logged error");
	states[frame]["svc"]["parachainLog"] = serde_json::json!({"#map": []});
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains(&format!("frame {frame}: svc.parachainLog[1] differs")), "{error}");
}

#[test]
fn rejected_state_balance_works() {
	replay::trace(include_str!("../fixtures/quint/blocks/rejected_state_balance_works.itf.json"))
		.expect("the rejection identifies the target and the below-used reason in Coretime's log");
}
