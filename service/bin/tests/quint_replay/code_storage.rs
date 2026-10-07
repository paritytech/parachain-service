use super::itf::replay;

#[test]
fn acquisition_boundaries_works() {
	replay::trace(include_str!(
		"../fixtures/quint/code_storage/acquisition_boundaries_works.itf.json"
	))
	.unwrap();
}

#[test]
fn announcement_boundaries_works() {
	replay::trace(include_str!(
		"../fixtures/quint/code_storage/announcement_boundaries_works.itf.json"
	))
	.unwrap();
}

#[test]
fn trap_oracle_errors() {
	let original: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/code_storage/acquisition_boundaries_works.itf.json"
	))
	.unwrap();
	let mut trace = original.clone();
	let frame = trace["states"]
		.as_array_mut()
		.unwrap()
		.iter_mut()
		.find(|s| s["replayHostTrap"] == true)
		.unwrap();
	frame["replayHostTrap"] = false.into();
	assert!(replay::document_trace(&trace).is_err());
	let mut trace = original;
	let frame = trace["states"]
		.as_array_mut()
		.unwrap()
		.iter_mut()
		.find(|s| s["replayHostTrap"] == true)
		.unwrap();
	frame["replayHostFree"] = serde_json::json!({"#bigint":"0"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("free balance differs"));
}
