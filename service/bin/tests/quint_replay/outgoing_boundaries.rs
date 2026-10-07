//! Exact host spendable-balance boundaries, with a Quint compatibility oracle.
use super::itf::replay;
use serde_json::{Value, json};

#[test]
fn adjacent_works() {
	for text in [
		include_str!("../fixtures/quint/outgoing_boundaries/below_works.itf.json"),
		include_str!("../fixtures/quint/outgoing_boundaries/exact_works.itf.json"),
		include_str!("../fixtures/quint/outgoing_boundaries/above_works.itf.json"),
	] {
		replay::trace(text).unwrap();
	}
}

#[test]
fn corrupted_outcomes_errors() {
	let trace: Value = serde_json::from_str(include_str!(
		"../fixtures/quint/outgoing_boundaries/exact_works.itf.json"
	))
	.unwrap();
	for field in ["replayHostFree", "replayTransfers"] {
		let mut bad = trace.clone();
		bad["states"][1][field] =
			if field == "replayTransfers" { json!([]) } else { json!({"#bigint": "1"}) };
		let error = replay::document_trace(&bad).unwrap_err();
		assert!(
			error.contains(if field == "replayTransfers" {
				"replayTransfers differs"
			} else {
				"mixed host free balance differs"
			}),
			"{error}"
		);
	}
}
