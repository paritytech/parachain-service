use super::itf::replay;

#[test]
fn head_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/storage/head_rejection_works.itf.json")).unwrap();
}
#[test]
fn kv_rollback_works() {
	replay::trace(include_str!("../fixtures/quint/storage/kv_rollback_works.itf.json")).unwrap();
}
#[test]
fn bucket_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/storage/bucket_rejection_works.itf.json"))
		.unwrap();
}
#[test]
fn endpoint_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/storage/endpoint_rejection_works.itf.json"))
		.unwrap();
}
#[test]
fn boundaries_works() {
	replay::trace(include_str!("../fixtures/quint/storage/boundaries_works.itf.json")).unwrap();
}
#[test]
fn incoming_credit_works() {
	replay::trace(include_str!("../fixtures/quint/storage/incoming_credit_works.itf.json"))
		.unwrap();
}
#[test]
fn later_bucket_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/storage/later_bucket_rejection_works.itf.json"))
		.unwrap();
}

#[test]
fn head_partial_effects_works() {
	replay::trace(include_str!("../fixtures/quint/storage/head_partial_effects_works.itf.json"))
		.unwrap();
}

#[test]
fn corrupted_oracle_errors() {
	let original: serde_json::Value =
		serde_json::from_str(include_str!("../fixtures/quint/storage/kv_rollback_works.itf.json"))
			.unwrap();
	for field in ["replayStorageFree", "replayStorageLogs"] {
		let mut bad = original.clone();
		bad["states"][1][field] = if field == "replayStorageFree" {
			serde_json::json!({"#bigint":"999"})
		} else {
			serde_json::json!([])
		};
		assert!(replay::document_trace(&bad).is_err(), "{field}");
	}
}

#[test]
fn invalid_metadata_errors() {
	let original: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/storage/head_rejection_works.itf.json"
	))
	.unwrap();
	let mut missing = original.clone();
	missing["states"][1].as_object_mut().unwrap().remove("replayStorageFree");
	assert!(replay::document_trace(&missing).unwrap_err().contains("every storage field"));
	for field in ["replayStorageBudget", "replayStorageFree"] {
		let mut bad = original.clone();
		bad["states"][1][field] = serde_json::json!({"#bigint":"-1"});
		assert!(replay::document_trace(&bad).unwrap_err().contains("out of"));
	}
	let mut bad = original;
	bad["states"][1]["replayFailedHeads"] = serde_json::json!([{"#bigint":"2"}]);
	assert!(replay::document_trace(&bad).unwrap_err().contains("invalid failed head"));
}

#[test]
fn registration_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/storage/registration_rejection_works.itf.json"))
		.unwrap();
}

#[test]
fn metadata_growth_works() {
	replay::trace(include_str!("../fixtures/quint/storage/metadata_growth_works.itf.json"))
		.unwrap();
}

#[test]
fn staging_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/storage/staging_rejection_works.itf.json"))
		.unwrap();
}

#[test]
fn staging_compact_boundary_works() {
	replay::trace(include_str!(
		"../fixtures/quint/storage/staging_compact_boundary_works.itf.json"
	))
	.unwrap();
}

#[test]
fn metadata_oracle_errors() {
	// A failed append must retain the prior nonempty staging buffer.
	let mut staging: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/storage/staging_rejection_works.itf.json"
	))
	.unwrap();
	staging["states"][3]["svc"]["stagedValidatorKeys"] = serde_json::json!([]);
	assert!(replay::document_trace(&staging).is_err());

	// The head-transition invariant must not treat a rejected forced head as applied.
	let original: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/storage/metadata_growth_works.itf.json"
	))
	.unwrap();
	let mut absent = original.clone();
	absent["states"][2].as_object_mut().unwrap().remove("replayFailedMessages");
	assert!(replay::document_trace(&absent).unwrap_err().contains("every frame"));
	let mut missing = original.clone();
	missing["states"][2]["replayFailedMessages"] = serde_json::json!([]);
	assert!(replay::document_trace(&missing).unwrap_err().contains("parent_head_continuity"));
	for pair in [
		serde_json::json!([{"#bigint":"9"}, {"#bigint":"0"}]),
		serde_json::json!([{"#bigint":"0"}, {"#bigint":"9"}]),
	] {
		let mut bad = original.clone();
		bad["states"][2]["replayFailedMessages"] = serde_json::json!([{"#tup":pair}]);
		assert!(replay::document_trace(&bad).unwrap_err().contains("invalid failed message"));
	}
}

#[test]
fn forced_code_partial_effects_works() {
	replay::trace(include_str!(
		"../fixtures/quint/storage/forced_code_partial_effects_works.itf.json"
	))
	.unwrap();
}
