//! Model integers must not alias Rust values through truncation or sign changes.

use serde_json::{json, Value};

use super::replay::{bounded_integer, document_trace};

fn fixture() -> Value {
	serde_json::from_str(include_str!(
		"../../fixtures/quint/log_pruning/stale_parent_seed_1_works.itf.json"
	))
	.unwrap()
}

fn rejects_change(pointer: &str, value: i128, field: &str, width: &str) {
	let mut trace = fixture();
	*trace.pointer_mut(pointer).expect("existing numeric field") =
		json!({"#bigint": value.to_string()});
	let error = document_trace(&trace).unwrap_err();
	assert!(error.contains(&format!("{field} out of {width} range: {value}")), "{error}");
}

#[test]
fn balance_range_errors() {
	// Frame zero exercises seeding; frame eleven exercises post-replay comparison.
	for frame in [0, 11] {
		for field in ["totalStateBalance", "usedStateBalance"] {
			let pointer = format!("/states/{frame}/svc/parachains/#map/0/1/{field}");
			let original = fixture().pointer(&pointer).unwrap()["#bigint"]
				.as_str()
				.unwrap()
				.parse::<i128>()
				.unwrap();
			for value in [-1, original + (1i128 << 64)] {
				rejects_change(&pointer, value, field, "u64");
			}
		}
	}
}

#[test]
fn block_slot_range_errors() {
	for value in [-1, 1i128 << 32] {
		rejects_change("/states/1/now", value, "now", "u32");
	}
}

#[test]
fn lookup_anchor_range_errors() {
	for value in [-1, 1i128 << 32] {
		rejects_change(
			"/states/1/lastStepWorkResults/0/result/value/value/lookupAnchor",
			value,
			"lookupAnchor",
			"u32",
		);
	}
}

#[test]
fn initial_preimage_length_range_errors() {
	for map in ["preimageRegistry", "preimageStatus"] {
		for value in [-1, (1i128 << 32) + 65_536] {
			rejects_change(
				&format!("/states/0/svc/{map}/#map/0/0/#tup/1"),
				value,
				"preimage length",
				"u32",
			);
		}
	}
}

#[test]
fn unsigned_boundaries_works() {
	for value in [0, u32::MAX] {
		assert_eq!(
			bounded_integer::<u32>(&json!({"#bigint": value.to_string()}), "test").unwrap(),
			value
		);
	}
	for value in [0, u64::MAX] {
		assert_eq!(
			bounded_integer::<u64>(&json!({"#bigint": value.to_string()}), "test").unwrap(),
			value
		);
	}
}
