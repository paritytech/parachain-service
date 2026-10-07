//! Host designation privilege as a generated invocation input.
use super::itf::replay;
use serde_json::{Value, json};

#[test]
fn privilege_changes_works() {
	for text in [
		include_str!("../fixtures/quint/designation/privilege_recovery_works.itf.json"),
		include_str!("../fixtures/quint/designation/abort_and_invalid_works.itf.json"),
		include_str!("../fixtures/quint/designation/full_set_works.itf.json"),
	] {
		replay::trace(text).unwrap();
	}
}

#[test]
fn corrupted_privilege_errors() {
	let mut trace: Value = serde_json::from_str(include_str!(
		"../fixtures/quint/designation/privilege_recovery_works.itf.json"
	))
	.unwrap();
	trace["states"][2]["replayCanDesignate"] = json!(true);
	assert!(replay::document_trace(&trace).is_err());
}

#[test]
fn missing_privilege_errors() {
	let mut trace: Value = serde_json::from_str(include_str!(
		"../fixtures/quint/designation/privilege_recovery_works.itf.json"
	))
	.unwrap();
	trace["states"][2].as_object_mut().unwrap().remove("replayCanDesignate");
	assert!(replay::document_trace(&trace).unwrap_err().contains("replayCanDesignate"));
}
