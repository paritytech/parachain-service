//! KV ordering, ownership, SCALE footprints, and failed reservations.

use crate::itf::replay;

#[test]
fn overwrite_and_refund_works() {
	replay::trace(include_str!("../fixtures/quint/kv/overwrite_and_refund_works.itf.json"))
		.expect("KV storage, balances, logs, and commitments should match Quint");
}

#[test]
fn delegated_removal_works() {
	replay::trace(include_str!("../fixtures/quint/kv/delegated_removal_works.itf.json"))
		.expect("KV storage, balances, logs, and commitments should match Quint");
}

#[test]
fn insufficient_balance_works() {
	replay::trace(include_str!("../fixtures/quint/kv/insufficient_balance_works.itf.json"))
		.expect("KV storage, balances, logs, and commitments should match Quint");
}

#[test]
fn stale_candidate_works() {
	replay::trace(include_str!("../fixtures/quint/kv/stale_candidate_works.itf.json"))
		.expect("KV storage, balances, logs, and commitments should match Quint");
}

#[test]
fn failure_message_id_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/kv/insufficient_balance_works.itf.json"
	))
	.unwrap();
	// Every logged event carries the `MessageId` of the upward message that caused
	// it (§5.1). Re-tagging one entry must make the strict log comparison fail.
	let mut retagged = false;
	for state in trace["states"].as_array_mut().unwrap() {
		// `parachainLog` is a map of para -> list of `(timeslot, LogEntry)` pairs.
		for entry in state["svc"]["parachainLog"]["#map"].as_array_mut().unwrap() {
			for slot_entry in entry[1].as_array_mut().unwrap() {
				let pair = slot_entry["#tup"].as_array_mut().expect("log entry is a pair");
				let Some(events) = pair[1]["value"].as_array_mut() else { continue };
				for event in events {
					let event = event["#tup"].as_array_mut().expect("event is a pair");
					if event[1]["tag"] == "InsufficientStateBalance" {
						event[0] = serde_json::json!({"#bigint": "99"});
						retagged = true;
					}
				}
			}
		}
	}
	assert!(retagged, "fixture must log a rejected KV write");
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("parachainLog"), "{error}");
}
