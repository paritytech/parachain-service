//! Upgrade provision, candidate-triggered activation, and lazy expiry.

use serde_json::{json, Value};

use crate::itf::replay;

const TRACE: &str =
	include_str!("../fixtures/quint/upgrades/work_error_across_upgrade_deadline_works.itf.json");
const ACTIVATION: &str = include_str!("../fixtures/quint/upgrades/activation_works.itf.json");

#[test]
fn insufficient_balance_preserves_pending_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/insufficient_balance_preserves_pending_works.itf.json"
	))
	.expect("a rejected upgrade logs the failed reservation and preserves the pending code");
}

// Known differences: keep these explicit while the fuzz runner continues to
// fail on them. See upstream-feedback/upgrade-expiry-replay.md.
#[test]
fn provided_upgrade_expiry_log_errors() {
	let error = replay::trace(include_str!(
		"../fixtures/quint/upgrades/provided_upgrade_expiry_log_works.itf.json"
	))
	.unwrap_err();
	assert!(error.contains("svc.parachainLog[1] differs"), "{error}");
	assert!(error.contains("Quint=[]") && error.contains("ForgetAgainAt"), "{error}");
}

#[test]
fn expired_code_candidate_reaps_errors() {
	let error = replay::trace(include_str!(
		"../fixtures/quint/upgrades/expired_code_candidate_reaps_works.itf.json"
	))
	.unwrap_err();
	assert!(error.contains("svc.parachains[1].pendingUpgrade differs"), "{error}");
}

#[test]
fn activation_works() {
	replay::trace(ACTIVATION).expect("the new-code candidate activates the provided upgrade");
}

fn unchanged_activation_field_errors(field: &str) {
	let mut trace: Value = serde_json::from_str(ACTIVATION).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.rfind(|&i| states[i]["now"] != states[i - 1]["now"])
		.expect("activation candidate block");
	let before = coretime_info(&mut states[frame - 1])[field].clone();
	assert_ne!(coretime_info(&mut states[frame])[field], before);
	coretime_info(&mut states[frame])[field] = before;
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(
		error.starts_with(&format!("frame {frame}: svc.parachains[1].{field} differs;")),
		"{error}"
	);
}

#[test]
fn activation_retains_old_code_errors() {
	unchanged_activation_field_errors("validationCode");
}

#[test]
fn activation_retains_pending_upgrade_errors() {
	unchanged_activation_field_errors("pendingUpgrade");
}

#[test]
fn work_error_across_upgrade_deadline_works() {
	replay::trace(TRACE).expect("WorkErr preserves the upgrade until a candidate reaps it");
}

fn coretime_info(frame: &mut Value) -> &mut Value {
	&mut frame["svc"]["parachains"]["#map"]
		.as_array_mut()
		.unwrap()
		.iter_mut()
		.find(|entry| entry[0]["value"]["#bigint"] == "1")
		.expect("Coretime is registered")[1]
}

#[test]
fn work_error_reaps_upgrade_errors() {
	let mut trace: Value = serde_json::from_str(TRACE).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.find(|&i| {
			states[i]["now"] != states[i - 1]["now"] &&
				states[i]["lastStepWorkResults"][0]["result"]["tag"] == "WorkErr"
		})
		.expect("WorkErr block past the deadline");
	let pending = &mut coretime_info(&mut states[frame])["pendingUpgrade"];
	assert_eq!(pending["tag"], "Some");
	*pending = json!({"tag": "None", "value": {"#tup": []}});
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(
		error.starts_with(&format!("frame {frame}: svc.parachains[1].pendingUpgrade differs;")),
		"{error}"
	);
}

#[test]
fn candidate_retains_expired_upgrade_errors() {
	let mut trace: Value = serde_json::from_str(TRACE).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.rfind(|&i| states[i]["now"] != states[i - 1]["now"])
		.expect("final candidate block");
	let pending = coretime_info(&mut states[frame - 1])["pendingUpgrade"].clone();
	assert_eq!(pending["tag"], "Some");
	assert_eq!(coretime_info(&mut states[frame])["pendingUpgrade"]["tag"], "None");
	coretime_info(&mut states[frame])["pendingUpgrade"] = pending;
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(
		error.starts_with(&format!("frame {frame}: svc.parachains[1].pendingUpgrade differs;")),
		"{error}"
	);
}
