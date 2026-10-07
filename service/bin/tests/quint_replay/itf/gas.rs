//! Replay gas metadata, kept separate from the pinned model's WorkResult type.
use super::replay::*;
use jam_types::AccumulateItem;
use serde_json::Value;

pub fn before_checkpoint(frame: &Value) -> Result<bool, String> {
	Ok(frame.get("replayInterrupt").map(integer).transpose()? == Some(-2))
}

// The report count itself selects exhaustion after the final checkpoint.
pub fn interrupted(frame: &Value) -> Result<Option<usize>, String> {
	match frame.get("replayInterrupt") {
		None => Ok(None),
		Some(value) => match integer(value)? {
			-2 | -1 => Ok(None),
			n => Ok(Some(usize::try_from(n).map_err(|_| "invalid replayInterrupt")?)),
		},
	}
}

pub fn limits(frame: &Value, count: usize) -> Result<Vec<u64>, String> {
	let Some(limits) = frame.get("replayGasLimits") else { return Ok(vec![u64::MAX; count]) };
	let limits = limits.as_array().ok_or("replayGasLimits must be a list")?;
	if limits.is_empty() && interrupted(frame)?.is_none() {
		return Ok(vec![u64::MAX; count]);
	}
	if limits.len() != count {
		return Err("replayGasLimits length must match work results".into());
	}
	if interrupted(frame)?.is_some_and(|index| index > count) {
		return Err("replayInterrupt is outside work results".into());
	}
	limits.iter().map(|v| bounded_integer(v, "report gas")).collect()
}

pub fn apply(frame: &Value, items: &mut [AccumulateItem]) -> Result<(), String> {
	let limits = limits(frame, items.len())?;
	for (item, limit) in items.iter_mut().zip(limits) {
		if let AccumulateItem::WorkItem(record) = item {
			record.gas_limit = limit;
		}
	}
	Ok(())
}

// Independently decode the specification budget for transition predicates.
pub(super) fn cost(result: &Value) -> Result<u64, String> {
	let (tag, digest) = variant(field(result, "result")?)?;
	if tag == "WorkErr" {
		return Ok(0);
	}
	let (tag, digest) = variant(digest)?;
	let mut cost = 5_000_000u64;
	if tag == "Ok" {
		for message in
			field(digest, "upwardMessages")?.as_array().ok_or("messages must be a list")?
		{
			cost = cost.saturating_add(250_000);
			let (tag, payload) = variant(message)?;
			if tag == "TransferOut" {
				let (tag, payload) = variant(field(payload, "deferred")?)?;
				if tag == "Some" {
					cost = cost.saturating_add(bounded_integer::<u64>(
						&tuple(payload)?[1],
						"forwarded gas",
					)?);
				}
			}
		}
	}
	Ok(cost)
}

pub fn effective(frame: &Value) -> Result<Value, String> {
	let mut effective = frame.clone();
	let results = field(frame, "lastStepWorkResults")?
		.as_array()
		.ok_or("work results must be a list")?;
	let limits = limits(frame, results.len())?;
	let end =
		if before_checkpoint(frame)? { 0 } else { interrupted(frame)?.unwrap_or(results.len()) };
	let mut applied = Vec::new();
	let mut failed_heads = Vec::new();
	for (i, result) in results.iter().enumerate().take(end) {
		if limits[i] >= cost(result)? {
			if let Some(heads) = frame.get("replayHostFailedHeads") {
				for head in heads.as_array().ok_or("expected failed host heads")? {
					if bounded_integer::<usize>(head, "failed host head")? == i {
						failed_heads
							.push(serde_json::json!({"#bigint": applied.len().to_string()}));
					}
				}
			}
			applied.push(result.clone());
		}
	}
	if frame.get("replayHostFailedHeads").is_some() {
		effective["replayHostFailedHeads"] = Value::Array(failed_heads);
	}
	effective["lastStepWorkResults"] = Value::Array(applied);
	Ok(effective)
}
