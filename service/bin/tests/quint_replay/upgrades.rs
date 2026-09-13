//! Lazy upgrade expiry runs on a candidate, even after WorkErr crossed the deadline.

use serde_json::{json, Value};

use crate::itf::replay;

const TRACE: &str =
	include_str!("../fixtures/quint/upgrades/work_error_across_upgrade_deadline_works.itf.json");

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
