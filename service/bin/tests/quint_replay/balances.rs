use super::itf::replay;

#[test]
fn balance_reservations_works() {
	replay::trace(include_str!("../fixtures/quint/balances/balance_reservations_works.itf.json"))
		.expect("balance updates, authorization, reservations, and refunds should match Quint");
}

#[test]
fn incoming_boundaries_works() {
	replay::trace(include_str!("../fixtures/quint/balances/incoming_boundaries_works.itf.json"))
		.expect("incoming admission, bucket rollover, and Asset Hub charges should match Quint");
}

#[test]
fn incoming_mutations_errors() {
	let fixture: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/balances/incoming_boundaries_works.itf.json"
	))
	.unwrap();
	// Change operands independently of expected state, and expected state
	// independently of operands. Both sides must participate in comparison.
	for pointer in [
		"/states/1/replayIncoming/0/amount/#bigint",
		"/states/1/replayIncoming/0/memo/#bigint",
		"/states/1/svc/incomingTransfers/#map/0/1/0/memo/#bigint",
		"/states/1/svc/incomingTransferBuckets/value/count/#bigint",
	] {
		let mut trace = fixture.clone();
		*trace.pointer_mut(pointer).expect("fixture field") = serde_json::json!("9999");
		assert!(replay::document_trace(&trace).is_err(), "accepted mutation at {pointer}");
	}
}

#[test]
fn dropped_transfer_operand_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/balances/incoming_boundaries_works.itf.json"
	))
	.unwrap();
	// The final incoming invocation is dropped. Raising its amount must make
	// Rust admit it and disagree with the unchanged model queue.
	let frame = trace["states"]
		.as_array_mut()
		.unwrap()
		.iter_mut()
		.rev()
		.find(|frame| frame["replayIncoming"].as_array().is_some_and(|v| !v.is_empty()))
		.unwrap();
	frame["replayIncoming"][0]["amount"] = serde_json::json!({"#bigint": "10000"});
	let error = replay::document_trace(&trace).unwrap_err();
	assert!(error.contains("differs"), "{error}");
}

#[test]
fn supervisor_transfer_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/balances/incoming_boundaries_works.itf.json"
	))
	.unwrap();
	trace["states"][1]["replayIncoming"][0]["toSupervisorBalance"] = serde_json::json!(true);
	assert!(replay::document_trace(&trace).unwrap_err().contains("supervisor-balance"));
}
