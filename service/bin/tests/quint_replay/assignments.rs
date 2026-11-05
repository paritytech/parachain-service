//! Assignment traces evaluated by the pinned model.
use super::itf::replay;

#[test]
fn inline_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/inline_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn delayed_boundary_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/delayed_boundary_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn queue_lengths_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/queue_lengths_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn replacement_order_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/replacement_order_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn authorization_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/authorization_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn invalid_queue_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/invalid_queue_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn handoff_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/handoff_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn rejected_work_flush_works() {
	replay::trace(include_str!("../fixtures/quint/assignments/rejected_work_flush_works.itf.json"))
		.expect("assignments should match Quint");
}

#[test]
fn future_assign_after_handoff_works() {
	replay::trace(include_str!(
		"../fixtures/quint/assignments/future_assign_after_handoff_works.itf.json"
	))
	.expect("assignments should match Quint");
}

#[test]
fn overwritten_call_history_errors() {
	let original: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/assignments/replacement_order_works.itf.json"
	))
	.unwrap();
	let frame = original["states"]
		.as_array()
		.unwrap()
		.iter()
		.position(|s| s["lastStepAssigns"].as_array().is_some_and(|a| a.len() == 3))
		.unwrap();
	for mutation in 0..3 {
		let mut changed = original.clone();
		let calls = changed["states"][frame]["lastStepAssigns"].as_array_mut().unwrap();
		match mutation {
			0 => {
				calls.remove(0);
			},
			1 => calls.swap(0, 1),
			_ => calls[0]["queue"][0]["authBytes"] = serde_json::json!({"#bigint":"999"}),
		}
		let error = replay::document_trace(&changed).unwrap_err();
		assert!(error.contains("ordered assignment calls differ"), "{error}");
	}
}

#[test]
fn independent_core_order_errors() {
	let mut trace: serde_json::Value =
		serde_json::from_str(include_str!("../fixtures/quint/assignments/inline_works.itf.json"))
			.unwrap();
	trace["states"][1]["lastStepAssigns"].as_array_mut().unwrap().swap(0, 1);
	let error = replay::document_trace(&trace).unwrap_err();
	assert!(error.contains("ordered assignment calls differ"), "{error}");
}
