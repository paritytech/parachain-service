//! Code-upgrade announcements, activation, and the two-phase lifecycle.

use serde_json::{json, Value};

use crate::itf::replay;

const ACTIVATION: &str = include_str!("../fixtures/quint/upgrades/activation_works.itf.json");
const WORK_ERROR: &str =
	include_str!("../fixtures/quint/upgrades/work_error_preserves_announcement_works.itf.json");
const SUPERSEDE: &str =
	include_str!("../fixtures/quint/upgrades/announcement_supersedes_previous_works.itf.json");

#[test]
fn activation_works() {
	replay::trace(ACTIVATION).expect("the applied candidate activates the announced code");
}

#[test]
fn provided_old_code_activation_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/provided_old_code_activation_works.itf.json"
	))
	.expect("activating provided code keeps its reference and charge until expunge");
}

#[test]
fn announcement_unavailable_rejected_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/announcement_unavailable_rejected_works.itf.json"
	))
	.expect("an unavailable announcement logs CodeUpgradeNotAvailable and stands no code");
}

#[test]
fn announcement_of_other_paras_code_rejected_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/announcement_of_other_paras_code_rejected_works.itf.json"
	))
	.expect("announcing a code the para does not reference logs CodeUpgradeNotAvailable");
}

#[test]
fn apply_without_announcement_rejected_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/apply_without_announcement_rejected_works.itf.json"
	))
	.expect("an Apply with no standing announcement logs CodeUpgradeNotAnnounced");
}

#[test]
fn apply_mismatched_announcement_rejected_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/apply_mismatched_announcement_rejected_works.itf.json"
	))
	.expect("an Apply naming code other than the announcement logs CodeUpgradeNotAnnounced");
}

#[test]
fn announcement_supersedes_previous_works() {
	replay::trace(SUPERSEDE).expect("a second announcement replaces the first, with no log");
}

#[test]
fn forget_announced_code_refused_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/forget_announced_code_refused_works.itf.json"
	))
	.expect("forgetting announced validation code logs CanNotForgetValidationCode and keeps it");
}

#[test]
fn insufficient_balance_preserves_announcement_works() {
	replay::trace(include_str!(
		"../fixtures/quint/upgrades/insufficient_balance_preserves_announcement_works.itf.json"
	))
	.expect("a rejected reservation preserves the standing announcement");
}

#[test]
fn work_error_preserves_announcement_works() {
	replay::trace(WORK_ERROR).expect("skipped work preserves the announcement until applied");
}

fn unchanged_activation_field_errors(field: &str) {
	let mut trace: Value = serde_json::from_str(ACTIVATION).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.rfind(|&i| {
			states[i]["now"] != states[i - 1]["now"] &&
				coretime_info_ref(&states[i])[field] != coretime_info_ref(&states[i - 1])[field]
		})
		.expect("activation candidate block");
	let before = coretime_info_ref(&states[frame - 1])[field].clone();
	assert_ne!(coretime_info_ref(&states[frame])[field], before);
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
fn activation_retains_announced_upgrade_errors() {
	unchanged_activation_field_errors("announcedUpgrade");
}

fn none() -> Value {
	json!({"tag": "None", "value": {"#tup": []}})
}

fn assert_announcement_differs(trace: &Value, frame: usize, context: &str) {
	let error = replay::trace(&trace.to_string()).unwrap_err();
	assert!(
		error.starts_with(&format!("frame {frame}: svc.parachains[1].announcedUpgrade differs;")),
		"{context}: {error}"
	);
}

#[test]
fn work_error_preserves_announcement_errors() {
	let mut trace: Value = serde_json::from_str(WORK_ERROR).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.find(|&i| {
			states[i]["now"] != states[i - 1]["now"] &&
				states[i]["lastStepWorkResults"][0]["result"]["tag"] == "WorkErr"
		})
		.expect("WorkErr block");
	assert_eq!(coretime_info(&mut states[frame])["announcedUpgrade"]["tag"], "Some");
	coretime_info(&mut states[frame])["announcedUpgrade"] = none();
	assert_announcement_differs(&trace, frame, "skipped work must preserve the announcement");
}

#[test]
fn announcement_supersedes_previous_errors() {
	let mut trace: Value = serde_json::from_str(SUPERSEDE).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let mut frame = None;
	for i in 1..states.len() {
		let before = coretime_info_ref(&states[i - 1])["announcedUpgrade"].clone();
		let after = coretime_info_ref(&states[i])["announcedUpgrade"].clone();
		if before != after && after["tag"] == "Some" {
			frame = Some(i);
		}
	}
	let frame = frame.expect("superseding announcement block");
	let before = coretime_info_ref(&states[frame - 1])["announcedUpgrade"].clone();
	coretime_info(&mut states[frame])["announcedUpgrade"] = before;
	assert_announcement_differs(&trace, frame, "the superseded announcement must be replaced");
}

fn retained_announcement_errors(trace_json: &str, message: &str, context: &str) {
	let mut trace: Value = serde_json::from_str(trace_json).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let frame = (1..states.len())
		.rfind(|&i| {
			states[i]["now"] != states[i - 1]["now"] &&
				states[i]["lastStepWorkResults"][0]["result"]["value"]["value"]["upwardMessages"]
					[0]["tag"] == message
		})
		.expect("candidate carrying the message");
	assert_eq!(
		coretime_info(&mut states[frame])["announcedUpgrade"]["tag"],
		"Some",
		"{context}: the announcement must stand"
	);
	coretime_info(&mut states[frame])["announcedUpgrade"] = none();
	assert_announcement_differs(&trace, frame, context);
}

#[test]
fn forget_announced_code_refused_retains_announcement_errors() {
	retained_announcement_errors(
		include_str!("../fixtures/quint/upgrades/forget_announced_code_refused_works.itf.json"),
		"Forget",
		"a refused forget must keep the announcement",
	);
}

#[test]
fn insufficient_balance_preserves_announcement_errors() {
	retained_announcement_errors(
		include_str!(
			"../fixtures/quint/upgrades/insufficient_balance_preserves_announcement_works.itf.json"
		),
		"Solicit",
		"a rejected reservation must keep the announcement",
	);
}

fn coretime_info(frame: &mut Value) -> &mut Value {
	&mut frame["svc"]["parachains"]["#map"]
		.as_array_mut()
		.unwrap()
		.iter_mut()
		.find(|entry| entry[0]["value"]["#bigint"] == "1")
		.expect("Coretime is registered")[1]
}

fn coretime_info_ref(frame: &Value) -> &Value {
	&frame["svc"]["parachains"]["#map"]
		.as_array()
		.unwrap()
		.iter()
		.find(|entry| entry[0]["value"]["#bigint"] == "1")
		.expect("Coretime is registered")[1]
}
