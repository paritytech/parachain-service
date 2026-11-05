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

const RECOVERY: [&str; 2] = [
	include_str!("../fixtures/quint/code_recovery/acquisition_recovery_works.itf.json"),
	include_str!("../fixtures/quint/code_recovery/announcement_recovery_works.itf.json"),
];

#[test]
fn sampled_recovery_works() {
	for fixture in RECOVERY {
		for sample in [0, 250000, 500000, 750000, 1000000] {
			let mut trace: serde_json::Value = serde_json::from_str(fixture).unwrap();
			for state in trace["states"].as_array_mut().unwrap().iter_mut().skip(1) {
				if state["replayInterrupt"] != serde_json::json!({"#bigint":"-1"}) &&
					state["replayHostTrap"] == false
				{
					state["replayGasSample"] = serde_json::json!({"#bigint":sample.to_string()});
				}
			}
			replay::document_trace(&trace).unwrap();
		}
	}
}

#[test]
fn lost_acquisition_checkpoint_errors() {
	let mut trace: serde_json::Value = serde_json::from_str(RECOVERY[0]).unwrap();
	let states = trace["states"].as_array_mut().unwrap();
	let index = states
		.iter()
		.position(|state| {
			state["replayCodeFailure"] == "forced-metadata" &&
				state["replayInterrupt"] == serde_json::json!({"#bigint":"1"})
		})
		.unwrap();
	assert_ne!(
		states[index]["svc"]["preimageRegistry"],
		states[index - 1]["svc"]["preimageRegistry"]
	);
	states[index]["svc"]["preimageRegistry"] = states[index - 1]["svc"]["preimageRegistry"].clone();
	assert!(replay::document_trace(&trace).is_err());
}
