use super::itf::replay;

#[test]
fn gas_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/host_mixed/gas_rejection_works.itf.json"))
		.unwrap();
}
#[test]
fn checkpoint_recovery_works() {
	replay::trace(include_str!("../fixtures/quint/host_mixed/checkpoint_recovery_works.itf.json"))
		.unwrap();
}
#[test]
fn due_assignment_recovery_works() {
	replay::trace(include_str!(
		"../fixtures/quint/host_mixed/due_assignment_recovery_works.itf.json"
	))
	.unwrap();
}
#[test]
fn release_then_retry_works() {
	replay::trace(include_str!("../fixtures/quint/host_mixed/release_then_retry_works.itf.json"))
		.unwrap();
}
#[test]
fn forget_then_retry_works() {
	replay::trace(include_str!("../fixtures/quint/host_mixed/forget_then_retry_works.itf.json"))
		.unwrap();
}
#[test]
fn code_upgrade_works() {
	replay::trace(include_str!("../fixtures/quint/host_mixed/code_upgrade_works.itf.json"))
		.unwrap();
}
#[test]
fn service_upgrade_works() {
	replay::trace(include_str!("../fixtures/quint/host_mixed/service_upgrade_works.itf.json"))
		.unwrap();
}
#[test]
fn queue_cleanup_works() {
	replay::trace(include_str!("../fixtures/quint/host_mixed/queue_cleanup_works.itf.json"))
		.unwrap();
}
#[test]
fn corrupted_budget_errors() {
	let original: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/host_mixed/checkpoint_recovery_works.itf.json"
	))
	.unwrap();
	let mut missing = original.clone();
	missing["states"][1].as_object_mut().unwrap().remove("replayHostFree");
	assert!(replay::document_trace(&missing).unwrap_err().contains("all three fields"));
	for (field, value, error) in [
		("replayHostBudget", "-2", "out of"),
		("replayHostFree", "-2", "out of"),
		("replayHostRejects", "-1", "out of"),
		("replayHostBudget", "18446744073709551615", "overflow"),
		("replayHostFree", "999999", "mixed host free balance differs"),
	] {
		let mut bad = original.clone();
		bad["states"][1][field] = serde_json::json!({"#bigint":value});
		assert!(replay::document_trace(&bad).unwrap_err().contains(error), "{field}");
	}
}

#[test]
fn cleanup_bucket_range_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/host_mixed/queue_cleanup_works.itf.json"
	))
	.unwrap();
	let message = &mut trace["states"][1]["lastStepWorkResults"][0]["result"]["value"]["value"]
		["upwardMessages"][1];
	assert_eq!(message["tag"], "CleanUpBucketsUpTo");
	message["value"] = serde_json::json!({"#bigint":"-1"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("cleanup bucket out of"));
}
