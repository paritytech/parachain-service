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
