//! Reuse Quint's explicit prefix oracle, replacing its panic with real gas loss.
use super::itf::replay;
use serde_json::{Value, json};

const FIXTURES: [&str; 3] = [
	include_str!("../fixtures/quint/panic_recovery/first_checkpoint_works.itf.json"),
	include_str!("../fixtures/quint/panic_recovery/middle_checkpoint_works.itf.json"),
	include_str!("../fixtures/quint/panic_recovery/last_checkpoint_works.itf.json"),
];

fn cutoff(stop: usize, key: &str, value: Value) -> Value {
	let mut trace: Value = serde_json::from_str(FIXTURES[stop]).unwrap();
	trace["states"][1]["replayPanic"] = json!(false);
	trace["states"][1][key] = value;
	trace
}

#[test]
fn report_interiors_works() {
	for stop in 0..3 {
		for sample in [0, 250_000, 500_000, 750_000, 1_000_000] {
			let trace = cutoff(stop, "replayGasSample", json!({"#bigint": sample.to_string()}));
			replay::document_trace(&trace)
				.unwrap_or_else(|e| panic!("report {stop}, sample {sample}: {e}"));
		}
	}
}

#[test]
fn host_boundaries_works() {
	for stop in 0..3 {
		for call in ["write", "new", "transfer"] {
			for side in ["before", "after"] {
				let boundary = format!("{side}:{call}");
				let trace = cutoff(stop, "replayGasBoundary", json!(boundary));
				replay::document_trace(&trace)
					.unwrap_or_else(|e| panic!("report {stop}, {boundary}: {e}"));
			}
		}
	}
}

#[test]
fn insufficient_invocation_gas_errors() {
	let trace = cutoff(0, "replayInvocationGas", json!({"#bigint": "0"}));
	assert!(replay::document_trace(&trace).unwrap_err().contains("before any checkpoint"));
}

#[test]
fn unlimited_invocation_gas_errors() {
	let trace = cutoff(0, "replayInvocationGas", json!({"#bigint": "5000000000"}));
	assert!(
		replay::document_trace(&trace)
			.unwrap_err()
			.contains("expected checkpoint NotEnoughGas")
	);
}

#[test]
fn wrong_prefix_errors() {
	let mut trace = cutoff(1, "replayGasSample", json!({"#bigint": "500000"}));
	trace["states"][1]["replayInterrupt"] = json!({"#bigint": "0"});
	assert!(replay::document_trace(&trace).unwrap_err().contains("headData differs"));
}

#[test]
fn invalid_sample_errors() {
	let trace = cutoff(0, "replayGasSample", json!({"#bigint": "1000001"}));
	assert!(replay::document_trace(&trace).unwrap_err().contains("gas sample must be"));
}

#[test]
fn retained_host_effects_errors() {
	for (key, expected) in [
		("replayTransfers", "replayTransfers differs"),
		("replayCreations", "creation set differs"),
	] {
		let mut trace = cutoff(1, "replayGasBoundary", json!("after:transfer"));
		assert!(!trace["states"][1][key].as_array().unwrap().is_empty());
		trace["states"][1][key] = json!([]);
		let error = replay::document_trace(&trace).unwrap_err();
		assert!(error.contains(expected), "{error}");
	}
}

#[test]
fn explicit_invocation_gas_works() {
	let mut trace: Value = serde_json::from_str(FIXTURES[1]).unwrap();
	trace["states"][2]["replayInvocationGas"] = json!({"#bigint": "5000000000"});
	replay::document_trace(&trace).unwrap();
}

#[test]
fn conflicting_faults_errors() {
	let mut trace = cutoff(0, "replayInvocationGas", json!({"#bigint": "1"}));
	trace["states"][1]["replayPanic"] = json!(true);
	assert!(replay::document_trace(&trace).unwrap_err().contains("cannot be combined"));
}

/// Deterministic decoration of mixed Quint campaigns. Keep half of the panic
/// frames as traps; sample invocation gas in eligible remaining frames. The
/// preserved failure envelope includes both the original oracle and the sample.
pub fn sample(trace: &mut Value, seed: u64) -> usize {
	let mut random = seed;
	let mut count = 0;
	for state in trace["states"].as_array_mut().unwrap() {
		random = random.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
		if state["replayInterrupt"]["#bigint"] == "-2" {
			state["replayGasSample"] = json!({"#bigint": ((random >> 33) % 1_000_001).to_string()});
			count += 1;
			continue;
		}
		if state["replayPanic"] != true || (random >> 32) % 2 == 0 {
			continue;
		}
		let stop = replay::integer(&state["replayInterrupt"]).unwrap() as usize;
		if !state["replayGasLimits"].as_array().is_some_and(|limits| {
			limits
				.iter()
				.enumerate()
				.all(|(i, l)| i == stop || l["#bigint"] == u64::MAX.to_string())
		}) {
			continue;
		}
		// The interrupted report contributes no model effects at either budget.
		// Admit it so exhaustion can happen inside its body, beyond the gas gate.
		state["replayGasLimits"][stop] = json!({"#bigint": u64::MAX.to_string()});
		state["replayPanic"] = json!(false);
		state["replayGasSample"] = json!({"#bigint": ((random >> 33) % 1_000_001).to_string()});
		count += 1;
	}
	count
}
