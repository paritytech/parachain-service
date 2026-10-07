//! Explicit fault injection for checkpoint replay. Ordinary VM errors still fail replay.
use super::{gas, replay::*};
use jam_types::{AccumulateItem, WorkOutput};
use serde_json::Value;

pub fn expected(frame: &Value) -> Result<Option<&'static str>, String> {
	let panic = frame.get("replayPanic").map(boolean).transpose()?.unwrap_or(false);
	if gas::before_checkpoint(frame)? {
		if panic {
			return Err("pre-checkpoint gas and panic injection cannot be combined".into());
		}
		return Ok(Some("NotEnoughGas"));
	}
	match (gas::interrupted(frame)?, panic) {
		(None, false) => Ok(None),
		(None, true) => Err("replayPanic requires replayInterrupt".into()),
		(Some(_), true) => Ok(Some("Trap")),
		(Some(_), false) => Ok(Some("NotEnoughGas")),
	}
}

pub fn inject(frame: &Value, items: &mut [AccumulateItem]) -> Result<(), String> {
	if expected(frame)? != Some("Trap") {
		return Ok(());
	}
	let index = gas::interrupted(frame)?.ok_or("missing panic report")?;
	let Some(AccumulateItem::WorkItem(record)) = items.get_mut(index) else {
		return Err("panic report is outside work results".into());
	};
	if record.result.is_err() {
		return Err("panic report must have WorkOk output".into());
	}
	// An empty SCALE digest reaches the production decode_all().expect(),
	// before any report writes or gas-gate checks. Do not replace the guest code.
	record.result = Ok(WorkOutput(Vec::new()));
	Ok(())
}
