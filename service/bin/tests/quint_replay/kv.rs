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
fn failure_key_hash_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(include_str!(
		"../fixtures/quint/kv/insufficient_balance_works.itf.json"
	))
	.unwrap();
	// Swap a real failed key for another registered key, keeping a valid codex
	// value: strict log comparison must detect the incorrect failure reason.
	fn corrupt(value: &mut serde_json::Value) -> bool {
		if let Some(hash) = value.get_mut("kvKeyBytes") {
			*hash = serde_json::json!({"#bigint": "4"});
			return true;
		}
		match value {
			serde_json::Value::Array(values) => values.iter_mut().any(corrupt),
			serde_json::Value::Object(values) => values.values_mut().any(corrupt),
			_ => false,
		}
	}
	assert!(corrupt(&mut trace["states"]));
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(error.contains("parachainLog"), "{error}");
}
