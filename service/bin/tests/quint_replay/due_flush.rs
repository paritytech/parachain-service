//! Funded flush faults use a hazmat gas drain; Quint fixes rollback and retry.
use super::itf::replay;
use serde_json::{json, Value};

const FIXTURES: [(&str, &str); 4] = [
	("initial", include_str!("../fixtures/quint/due_flush/initial_tiling_works.itf.json")),
	("initial", include_str!("../fixtures/quint/due_flush/initial_rotating_works.itf.json")),
	("final", include_str!("../fixtures/quint/due_flush/final_tiling_works.itf.json")),
	("final", include_str!("../fixtures/quint/due_flush/final_rotating_works.itf.json")),
];

fn trace(fixture: &str, boundary: &str) -> Value {
	let mut trace: Value = serde_json::from_str(fixture).unwrap();
	trace["states"][2]["replayDueFlushBoundary"] = json!(boundary);
	trace
}

#[test]
fn host_boundaries_works() {
	for (phase, fixture) in FIXTURES {
		for (call, minimum) in [("assign", 3), ("write", 4), ("read", 1)] {
			let mut count = 0;
			for occurrence in 0..128 {
				let mut document = trace(fixture, &format!("{phase}:before:{call}:{occurrence}"));
				match replay::document_trace(&document) {
					Err(error) if error.contains("due-flush boundary call absent") => break,
					result => {
						result.unwrap_or_else(|e| panic!("{phase} before {call}:{occurrence}: {e}"))
					},
				}
				document["states"][2]["replayDueFlushBoundary"] =
					json!(format!("{phase}:after:{call}:{occurrence}"));
				replay::document_trace(&document)
					.unwrap_or_else(|e| panic!("{phase} after {call}:{occurrence}: {e}"));
				count += 1;
			}
			assert!((minimum..128).contains(&count), "{phase} {call}: {count}");
			if call == "assign" {
				assert_eq!(count, 3, "all three due cores must be reached");
			}
		}
	}
}

#[test]
fn retained_assignment_errors() {
	for (phase, fixture) in FIXTURES {
		let mut document = trace(fixture, &format!("{phase}:after:assign:2"));
		document["states"][2]["lastStepAssigns"] = document["states"][3]["lastStepAssigns"].clone();
		assert_eq!(document["states"][2]["lastStepAssigns"].as_array().unwrap().len(), 3);
		assert!(replay::document_trace(&document).unwrap_err().contains("assign"));
	}
}

#[test]
fn invalid_boundary_errors() {
	let fixture = FIXTURES[0].1;
	for (boundary, expected) in [
		("final:before:assign:0", "checkpoint expectation"),
		("initial:after:assign:3", "call absent"),
		("initial:after:checkpoint:0", "unsupported"),
		("initial:after:assign:-1", "invalid due-flush occurrence"),
	] {
		assert!(replay::document_trace(&trace(fixture, boundary))
			.unwrap_err()
			.contains(expected));
	}
	let mut document = trace(fixture, "initial:before:assign:0");
	document["states"][2]["replayGasSample"] = json!({"#bigint": "0"});
	assert!(replay::document_trace(&document).unwrap_err().contains("cannot be combined"));
	let mut document = trace(fixture, "initial:before:assign:0");
	document["states"][0]["replayDueFlushBoundary"] = json!("initial:before:assign:0");
	assert!(replay::document_trace(&document).unwrap_err().contains("initial frame"));
}

#[test]
fn cutoff_scope_works() {
	use jam_node::vm::replay_fault::with_cutoff;
	// Neither an unreachable boundary nor unwinding may contaminate later runs.
	let document = trace(FIXTURES[0].1, "");
	let (result, reached) = with_cutoff(usize::MAX, false, || replay::document_trace(&document));
	result.unwrap();
	assert!(!reached);
	assert!(std::panic::catch_unwind(|| with_cutoff(0, false, || panic!("scope unwind"))).is_err());
	replay::document_trace(&trace(FIXTURES[0].1, "initial:after:assign:0")).unwrap();
}

#[test]
fn fixtures_works() {
	for (_, fixture) in FIXTURES {
		replay::trace(fixture).unwrap();
	}
}
