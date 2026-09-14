mod blocks;
mod fuzz;
#[path = "../common/mod.rs"]
mod common;
mod itf;
mod log_pruning;
mod refine_errors;
mod upgrades;

#[test]
fn minimal_replay_works() {
	itf::replay::trace(include_str!("../fixtures/quint/minimal_replay.itf.json"))
		.expect("Quint and Rust state should agree after every frame");
}

#[test]
fn stale_parent_candidate_rejected_works() {
	itf::replay::trace(include_str!("../fixtures/quint/staleParentCandidateRejectedTest.itf.json"))
		.expect("Quint and Rust state should agree after every frame");
}

#[test]
fn refine_error_replay_works() {
	itf::replay::trace(include_str!("../fixtures/quint/refine_error_replay.itf.json"))
		.expect("a logged RefineLogEntry and a gray-paper work error should both replay");
}
