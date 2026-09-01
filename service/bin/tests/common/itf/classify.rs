use std::collections::BTreeSet;

use serde_json::Value;

/// The four transition kinds understood by the replay harness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
	Block,
	IncomingTransfer,
	ProvisionPreimage,
	Noop,
}

/// Classify the transition from `previous` to `current`.
///
/// MBT traces name the action explicitly. Normalized `quint test` traces are
/// classified from the state changes, with ambiguity treated as an error.
pub fn classify(previous: &Value, current: &Value) -> Result<FrameKind, String> {
	if let Some(action) = current.get("mbt::actionTaken") {
		let action = action.as_str().ok_or("mbt::actionTaken must be a string")?;
		return match action {
			"stepRefineAccumulate" => Ok(FrameKind::Block),
			"stepIncomingTransfer" => Ok(FrameKind::IncomingTransfer),
			"provisionPreimage" => Ok(FrameKind::ProvisionPreimage),
			other => Err(format!("unsupported MBT action: {other}")),
		};
	}

	if previous == current {
		return Ok(FrameKind::Noop);
	}

	let work_results = current
		.get("lastStepWorkResults")
		.and_then(Value::as_array)
		.ok_or("frame must contain a lastStepWorkResults array")?;
	let now_unchanged = previous.get("now") == current.get("now");
	let changed_svc_fields = changed_svc_fields(previous, current)?;

	// Only stepRefineAccumulate advances `now`, so it catches zero-package blocks too.
	let block = !work_results.is_empty() || !now_unchanged;
	let provision = work_results.is_empty() &&
		now_unchanged &&
		changed_svc_fields == BTreeSet::from(["preimageStatus"]);
	let incoming = work_results.is_empty() &&
		now_unchanged &&
		!changed_svc_fields.is_empty() &&
		changed_svc_fields.iter().all(|field| {
			matches!(*field, "incomingTransfers" | "incomingTransferBuckets" | "parachains")
		});

	let matches = block as u8 + provision as u8 + incoming as u8;
	if matches != 1 {
		return Err(format!(
			"frame classification matched {matches} kinds (now_unchanged={now_unchanged}, \
			 work_results={}, changed_svc_fields={changed_svc_fields:?})",
			work_results.len()
		));
	}

	if block {
		Ok(FrameKind::Block)
	} else if provision {
		Ok(FrameKind::ProvisionPreimage)
	} else {
		Ok(FrameKind::IncomingTransfer)
	}
}

fn changed_svc_fields<'a>(
	previous: &'a Value,
	current: &'a Value,
) -> Result<BTreeSet<&'a str>, String> {
	let previous = previous
		.get("svc")
		.and_then(Value::as_object)
		.ok_or("previous frame has no svc record")?;
	let current = current
		.get("svc")
		.and_then(Value::as_object)
		.ok_or("current frame has no svc record")?;
	let keys = previous
		.keys()
		.chain(current.keys())
		.map(String::as_str)
		.collect::<BTreeSet<_>>();
	Ok(keys.into_iter().filter(|key| previous.get(*key) != current.get(*key)).collect())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn frame(now: u32, svc: Value, work_results: Value) -> Value {
		json!({"now": now, "svc": svc, "lastStepWorkResults": work_results})
	}

	fn svc() -> Value {
		json!({
			"parachains": {},
			"incomingTransfers": {},
			"incomingTransferBuckets": null,
			"preimageStatus": {}
		})
	}

	#[test]
	fn mbt_action_takes_precedence_works() {
		let previous = frame(0, svc(), json!([]));
		let mut current = previous.clone();
		current["mbt::actionTaken"] = json!("stepRefineAccumulate");
		assert_eq!(classify(&previous, &current).unwrap(), FrameKind::Block);
	}

	#[test]
	fn exact_noop_works() {
		let frame = frame(0, svc(), json!([]));
		assert_eq!(classify(&frame, &frame).unwrap(), FrameKind::Noop);
	}

	#[test]
	fn block_with_results_works() {
		let previous = frame(0, svc(), json!([]));
		let current = frame(1, svc(), json!([{"result": "anything"}]));
		assert_eq!(classify(&previous, &current).unwrap(), FrameKind::Block);
	}

	#[test]
	fn always_accumulate_block_works() {
		let previous = frame(0, svc(), json!([]));
		let current = frame(1, svc(), json!([]));
		assert_eq!(classify(&previous, &current).unwrap(), FrameKind::Block);
	}

	#[test]
	fn provision_preimage_works() {
		let previous = frame(0, svc(), json!([]));
		let mut changed = svc();
		changed["preimageStatus"]["hash"] = json!("Provided");
		let current = frame(0, changed, json!([]));
		assert_eq!(classify(&previous, &current).unwrap(), FrameKind::ProvisionPreimage);
	}

	#[test]
	fn incoming_transfer_works() {
		let previous = frame(0, svc(), json!([]));
		let mut changed = svc();
		changed["incomingTransfers"]["0"] = json!([100]);
		changed["incomingTransferBuckets"] = json!(0);
		changed["parachains"]["2"] = json!({"usedStateBalance": 10});
		let current = frame(0, changed, json!([]));
		assert_eq!(classify(&previous, &current).unwrap(), FrameKind::IncomingTransfer);
	}

	#[test]
	fn ambiguous_structural_frame_errors() {
		let previous = frame(0, svc(), json!([]));
		let mut changed = svc();
		changed["preimageStatus"]["hash"] = json!("Provided");
		changed["incomingTransfers"]["0"] = json!([100]);
		let current = frame(0, changed, json!([]));
		assert!(classify(&previous, &current).unwrap_err().contains("matched 0 kinds"));
	}

	#[test]
	fn unsupported_mbt_action_errors() {
		let previous = frame(0, svc(), json!([]));
		let mut current = previous.clone();
		current["mbt::actionTaken"] = json!("surpriseAction");
		assert!(classify(&previous, &current).unwrap_err().contains("surpriseAction"));
	}

	#[test]
	fn committed_fixture_block_works() {
		let fixture: Value =
			serde_json::from_str(include_str!("../../fixtures/quint/minimal_replay.itf.json"))
				.unwrap();
		let states = fixture["states"].as_array().unwrap();
		assert_eq!(classify(&states[0], &states[1]).unwrap(), FrameKind::Block);
	}
}
