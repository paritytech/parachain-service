#[test]
fn boundaries_works() {
	super::itf::replay::trace(include_str!("../fixtures/quint/gas/boundaries_works.itf.json"))
		.unwrap();
}

#[test]
fn first_checkpoint_works() {
	super::itf::replay::trace(include_str!(
		"../fixtures/quint/gas/first_checkpoint_works.itf.json"
	))
	.unwrap();
}

#[test]
fn middle_checkpoint_works() {
	super::itf::replay::trace(include_str!(
		"../fixtures/quint/gas/middle_checkpoint_works.itf.json"
	))
	.unwrap();
}

#[test]
fn last_checkpoint_works() {
	super::itf::replay::trace(include_str!("../fixtures/quint/gas/last_checkpoint_works.itf.json"))
		.unwrap();
}

#[test]
fn rejected_then_accepted_works() {
	super::itf::replay::trace(include_str!(
		"../fixtures/quint/gas/rejected_then_accepted_works.itf.json"
	))
	.unwrap();
}

#[test]
fn refine_error_budget_works() {
	super::itf::replay::trace(include_str!(
		"../fixtures/quint/gas/refine_error_budget_works.itf.json"
	))
	.unwrap();
}

#[test]
fn assignment_checkpoint_works() {
	super::itf::replay::trace(include_str!(
		"../fixtures/quint/gas/assignment_checkpoint_works.itf.json"
	))
	.unwrap();
}

#[test]
fn invalid_metadata_errors() {
	use serde_json::{json, Value};
	let trace: Value = serde_json::from_str(include_str!(
		"../fixtures/quint/gas/middle_checkpoint_works.itf.json"
	))
	.unwrap();
	for (field, value, expected) in [
		("replayGasLimits", json!([{"#bigint":"1"}]), "length"),
		("replayGasLimits", json!([{"#bigint":"-1"}, {"#bigint":"0"}, {"#bigint":"0"}]), "range"),
		("replayInterrupt", json!({"#bigint":"3"}), "outside"),
		("replayInterrupt", json!({"#bigint":"-2"}), "invalid"),
		("replayInterrupt", json!({"#bigint":"-1"}), "out of gas"),
	] {
		let mut bad = trace.clone();
		bad["states"][1][field] = value;
		let error = super::itf::replay::document_trace(&bad).unwrap_err();
		assert!(error.contains(expected), "{field}: {error}");
	}
	let mut bad = trace;
	bad["states"][1].as_object_mut().unwrap().remove("replayGasLimits");
	assert!(super::itf::replay::document_trace(&bad).unwrap_err().contains("every frame"));
}

#[test]
fn saturating_budget_works() {
	super::itf::replay::trace(include_str!(
		"../fixtures/quint/gas/saturating_budget_works.itf.json"
	))
	.unwrap();
}
