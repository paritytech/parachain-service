mod assignments;
mod balances;
mod blocks;
#[path = "../common/mod.rs"]
mod common;
mod fuzz;
mod gas;
mod storage;
mod itf;
mod kv;
mod lifecycle;
mod log_pruning;
mod mixed;
mod outgoing;
mod service_upgrades;
mod refine_errors;
mod upgrades;
mod validator_keys;

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
