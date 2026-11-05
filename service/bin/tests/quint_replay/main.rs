mod assignments;
mod balances;
mod blocks;
mod code_storage;
#[path = "../common/mod.rs"]
mod common;
mod designation;
mod fuzz;
mod gas;
mod host_mixed;
mod instruction_gas;
mod itf;
mod kv;
mod lifecycle;
mod log_pruning;
mod mixed;
mod outgoing;
mod outgoing_boundaries;
mod panic_recovery;
mod refine_errors;
mod self_payments;
mod service_management;
mod service_preimages;
mod service_upgrades;
mod services;
mod storage;
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
		.expect("a logged Refine and a gray-paper work error should both replay");
}

mod final_checkpoint;
mod pre_checkpoint;

mod refine;

mod combined;

mod storage_recovery;

mod host_calls;

mod authorization;

mod write_recovery;
mod due_flush;
