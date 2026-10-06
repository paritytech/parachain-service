use super::itf::replay;

#[test]
fn allocation_works() {
	replay::trace(include_str!("../fixtures/quint/services/allocation_works.itf.json")).unwrap();
}
#[test]
fn protected_collision_works() {
	replay::trace(include_str!("../fixtures/quint/services/protected_collision_works.itf.json"))
		.unwrap();
}
#[test]
fn balance_selectors_works() {
	replay::trace(include_str!("../fixtures/quint/services/balance_selectors_works.itf.json"))
		.unwrap();
}
#[test]
fn ejection_refusals_works() {
	replay::trace(include_str!("../fixtures/quint/services/ejection_refusals_works.itf.json"))
		.unwrap();
}
#[test]
fn gas_rejection_works() {
	replay::trace(include_str!("../fixtures/quint/services/gas_rejection_works.itf.json")).unwrap();
}
#[test]
fn checkpoint_first_works() {
	replay::trace(include_str!("../fixtures/quint/services/checkpoint_first_works.itf.json"))
		.unwrap();
}
#[test]
fn checkpoint_middle_works() {
	replay::trace(include_str!("../fixtures/quint/services/checkpoint_middle_works.itf.json"))
		.unwrap();
}
#[test]
fn checkpoint_last_works() {
	replay::trace(include_str!("../fixtures/quint/services/checkpoint_last_works.itf.json"))
		.unwrap();
}
#[test]
fn unauthorized_works() {
	replay::trace(include_str!("../fixtures/quint/services/unauthorized_works.itf.json")).unwrap();
}

#[test]
fn host_balance_works() {
	replay::trace(include_str!("../fixtures/quint/services/host_balance_works.itf.json")).unwrap();
}

#[test]
fn corrupted_creation_oracle_errors() {
	let original: serde_json::Value =
		serde_json::from_str(include_str!("../fixtures/quint/services/allocation_works.itf.json"))
			.unwrap();
	for (path, value, error) in [
		(
			"/states/1/replayCreations/0/args/minItemGas",
			serde_json::json!({"#bigint":"999"}),
			"created account",
		),
		(
			"/states/1/replayCreations/0/args/minMemoGas",
			serde_json::json!({"#bigint":"999"}),
			"created account",
		),
		(
			"/states/1/replayCreations/0/service/value",
			serde_json::json!({"#bigint":"99"}),
			"creation set differs",
		),
		("/states/1/replayCreations", serde_json::json!([]), "creation set differs"),
		(
			"/states/1/replayCreations/0/args/sourceSupervisorBalance",
			serde_json::json!(true),
			"cannot use supervisor balances",
		),
	] {
		let mut bad = original.clone();
		*bad.pointer_mut(path).unwrap() = value;
		assert!(replay::document_trace(&bad).unwrap_err().contains(error), "{path}");
	}

	let mut removed = original.clone();
	removed["states"][1]["foreignServices"]["#map"]
		.as_array_mut()
		.unwrap()
		.retain(|entry| entry[0]["value"]["#bigint"] != "7");
	assert!(replay::document_trace(&removed).unwrap_err().contains("creation set differs"));
	let mut missing = original.clone();
	for state in missing["states"].as_array_mut().unwrap() {
		state.as_object_mut().unwrap().remove("replayCreations");
	}
	assert!(replay::document_trace(&missing)
		.unwrap_err()
		.contains("require replayCreations"));
	let mut missing = original.clone();
	missing["states"][1].as_object_mut().unwrap().remove("replayCreations");
	assert!(replay::document_trace(&missing).unwrap_err().contains("every frame"));
}

#[test]
fn creation_numeric_bounds_errors() {
	let original: serde_json::Value =
		serde_json::from_str(include_str!("../fixtures/quint/services/allocation_works.itf.json"))
			.unwrap();
	for (name, number) in [
		("len", "-1"),
		("len", "4294967296"),
		("id", "-1"),
		("id", "18446744073709551616"),
		("minItemGas", "-1"),
		("minMemoGas", "18446744073709551616"),
	] {
		let mut bad = original.clone();
		bad["states"][1]["lastStepWorkResults"][0]["result"]["value"]["value"]["upwardMessages"]
			[0]["value"][name] = serde_json::json!({"#bigint":number});
		assert!(replay::document_trace(&bad).unwrap_err().contains("out of"), "{name}");
	}
}

#[test]
fn code_and_gas_boundaries_works() {
	replay::trace(include_str!(
		"../fixtures/quint/services/code_and_gas_boundaries_works.itf.json"
	))
	.unwrap();
}
