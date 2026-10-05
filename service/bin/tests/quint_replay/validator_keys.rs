//! Validator-key traces evaluated by the pinned model.
use super::itf::replay;

#[test]
fn chunks_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/chunks_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn same_set_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/same_set_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn abort_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/abort_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn invalid_lengths_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/invalid_lengths_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn full_set_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/full_set_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn overflow_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/overflow_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn authorization_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/authorization_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn stale_work_works() {
	replay::trace(include_str!("../fixtures/quint/validator_keys/stale_work_works.itf.json"))
		.expect("validator keys should match Quint");
}

#[test]
fn replacement_order_works() {
	replay::trace(include_str!(
		"../fixtures/quint/validator_keys/replacement_order_works.itf.json"
	))
	.expect("validator keys should match Quint");
}

#[test]
fn rejected_final_after_success_works() {
	replay::trace(include_str!(
		"../fixtures/quint/validator_keys/rejected_final_after_success_works.itf.json"
	))
	.expect("validator keys should match Quint");
}

#[test]
fn missing_designation_oracle_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/validator_keys/same_set_works.itf.json"
	))
	.unwrap();
	trace["states"][1].as_object_mut().unwrap().remove("replayDesignate");
	assert!(replay::document_trace(&trace).unwrap_err().contains("require replayDesignate"));
}
