//! Deferred self-payments debit immediately and receive scheduler credit later.
use super::itf::replay;
use serde_json::{Value, json};

#[test]
fn ordered_works() {
	replay::trace(include_str!("../fixtures/quint/self_payments/ordered_works.itf.json")).unwrap();
}

#[test]
fn recovery_works() {
	replay::trace(include_str!("../fixtures/quint/self_payments/recovery_works.itf.json")).unwrap();
}

#[test]
fn immediate_credit_errors() {
	let mut trace: Value = serde_json::from_str(include_str!(
		"../fixtures/quint/self_payments/ordered_works.itf.json"
	))
	.unwrap();
	// At the exact boundary, an immediate self-credit would incorrectly fund
	// this second payment within the same invocation.
	let message =
		trace["states"][2]["lastStepWorkResults"][1]["result"]["value"]["value"]["upwardMessages"]
			[1]["value"]
			.clone();
	trace["states"][2]["replayTransfers"].as_array_mut().unwrap().push(message);
	assert!(replay::document_trace(&trace).unwrap_err().contains("replayTransfers differs"));
}

#[test]
fn lost_delivery_errors() {
	let mut trace: Value = serde_json::from_str(include_str!(
		"../fixtures/quint/self_payments/recovery_works.itf.json"
	))
	.unwrap();
	let frame = &mut trace["states"][1];
	let balance = replay::integer(&frame["svc"]["jamAccount"]["balance"]).unwrap();
	let amount = replay::integer(&frame["replayTransfers"][0]["amount"]).unwrap();
	frame["svc"]["jamAccount"]["balance"] = json!({"#bigint": (balance - amount).to_string()});
	assert!(replay::document_trace(&trace).unwrap_err().contains("JAM account"));
}
