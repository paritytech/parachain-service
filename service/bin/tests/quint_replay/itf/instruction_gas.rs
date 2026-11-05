//! Locate instruction cutoffs using host-call order, never recovered state.
//!
//! Quint has already chosen the recovery phase and report prefix. A successful,
//! discarded run and gas-only probes locate its interval in the production VM.
//! The final run must still match every Quint state/effect assertion.
use std::{cell::RefCell, sync::OnceLock};

use jam_codec::{Decode, Encode};
use jam_node::vm::{AccumulateCallContext, Engine};
use jam_types::AccumulateItem;
use serde_json::Value;

use super::{gas, replay::*};

thread_local! {
	static EVENTS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

struct Observer;
impl log::Log for Observer {
	fn enabled(&self, metadata: &log::Metadata) -> bool {
		(metadata.target() == "service" && EVENTS.with(|events| events.borrow().is_some())) ||
			(metadata.target() == "replay_host" && super::host_calls::enabled())
	}
	fn log(&self, record: &log::Record) {
		if !self.enabled(record.metadata()) {
			return;
		}
		if record.target() == "replay_host" {
			super::host_calls::record(&record.args().to_string());
			return;
		}
		EVENTS.with(|events| {
			if let Some(events) = events.borrow_mut().as_mut() {
				if let Some(name) = record.args().to_string().strip_prefix("Handling hostcall: ") {
					events.push(name.into());
				}
			}
		});
	}
	fn flush(&self) {}
}

pub(super) fn observe() -> Result<(), String> {
	static INSTALLED: OnceLock<Result<(), String>> = OnceLock::new();
	INSTALLED
		.get_or_init(|| {
			log::set_logger(&Observer).map_err(|e| format!("gas observer: {e}"))?;
			log::set_max_level(log::LevelFilter::Trace);
			Ok(())
		})
		.clone()
}

struct Capture;
impl Drop for Capture {
	fn drop(&mut self) {
		EVENTS.with(|events| *events.borrow_mut() = None);
	}
}

fn probe(
	engine: &Engine,
	original: &AccumulateCallContext<'_>,
	limit: u64,
) -> Result<(Vec<String>, u64, bool), String> {
	observe()?;
	let mut context = parachain_service_bin::mock::accumulate_context_with_privileges(
		original.storage.clone(),
		Vec::decode(&mut &original.items.encode()[..]).map_err(|e| e.to_string())?,
		original.slot,
		original.privileges.clone(),
	);
	context.gas = limit;
	let code = context
		.storage
		.service(context.service_id)
		.ok_or("missing gas-probe service")?
		.code_hash;
	EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
	let _capture = Capture;
	let (result, _, used) = engine.accumulate(code, &mut context);
	if let Err(error) = &result {
		if format!("{error:?}") != "NotEnoughGas" {
			return Err(format!("instruction gas probe failed: {error:?}"));
		}
	}
	let events = EVENTS.with(|events| events.borrow_mut().take().unwrap());
	Ok((events, used, result.is_ok()))
}

/// A funded flush cannot be sampled by lowering entry gas: that changes the
/// flush budget itself. Inject exhaustion at a host boundary instead, preserving
/// the unmodified guest and its gas gates. Only call order comes from the probe.
pub fn due_flush_enabled(frame: &Value) -> bool {
	frame
		.get("replayDueFlushBoundary")
		.is_some_and(|value| value.as_str() != Some(""))
}

pub fn due_flush_cutoff(
	engine: &Engine,
	context: &AccumulateCallContext<'_>,
	frame: &Value,
) -> Result<Option<(u64, usize, bool)>, String> {
	let Some(boundary) = frame.get("replayDueFlushBoundary") else { return Ok(None) };
	if boundary.as_str() == Some("") {
		return Ok(None);
	}
	if enabled(frame) || frame.get("replayPanic") == Some(&Value::Bool(true)) {
		return Err("due-flush injection cannot be combined with another gas or panic fault".into());
	}
	let parts: Vec<_> = boundary
		.as_str()
		.ok_or("due-flush boundary must be a string")?
		.split(':')
		.collect();
	let [phase, side, call, occurrence] = parts.as_slice() else {
		return Err("expected initial|final:before|after:assign|write|read:occurrence".into());
	};
	if !matches!(*side, "before" | "after") || !matches!(*call, "assign" | "write" | "read") {
		return Err("unsupported due-flush boundary".into());
	}
	let occurrence = occurrence.parse::<usize>().map_err(|_| "invalid due-flush occurrence")?;
	let results = field(frame, "lastStepWorkResults")?.as_array().ok_or("missing work results")?;
	let initial = match *phase {
		"initial" if gas::before_checkpoint(frame)? => true,
		"final" if gas::interrupted(frame)? == Some(results.len()) => false,
		_ => return Err("due-flush phase disagrees with checkpoint expectation".into()),
	};
	if results.iter().zip(gas::limits(frame, results.len())?).any(|(r, l)| {
		variant(&r["result"]).map(|(tag, _)| tag != "WorkOk").unwrap_or(true) || l != u64::MAX
	}) {
		return Err("due-flush injection requires admitted WorkOk reports".into());
	}
	let declared = context.items.iter().fold(0u64, |sum, item| {
		sum.saturating_add(match item {
			AccumulateItem::WorkItem(record) => record.gas_limit,
			AccumulateItem::Transfer(transfer) => transfer.gas_limit,
		})
	});
	// For the final flush, keep the initial allowance below one assignment.
	let limit =
		if initial { context.gas } else { context.gas.min(declared.saturating_add(5_999_999)) };
	let (events, _, completed) = probe(engine, context, limit)?;
	if !completed {
		return Err("due-flush calibration did not complete".into());
	}
	let checkpoints: Vec<_> = events
		.iter()
		.enumerate()
		.filter_map(|(i, e)| (e == "checkpoint").then_some(i))
		.collect();
	if checkpoints.len() != results.len() + 1 {
		return Err("due-flush checkpoint count differs".into());
	}
	let (start, end) =
		if initial { (0, checkpoints[0]) } else { (checkpoints[results.len()] + 1, events.len()) };
	let assignments: Vec<_> = (start..end).filter(|&i| events[i] == "assign").collect();
	if assignments.is_empty() {
		return Err("selected phase has no due assignment".into());
	}
	if !initial && events[..checkpoints[0]].iter().any(|e| e == "assign") {
		return Err("final-flush calibration unexpectedly flushed initial assignments".into());
	}
	let index = (start..end)
		.filter(|&i| events[i] == *call)
		.nth(occurrence)
		.ok_or("due-flush boundary call absent from selected phase")?;
	Ok(Some((limit, index, *side == "after")))
}

// Minimum gas which reaches the selected host-call attempt. The call itself
// may run out of gas; reaching its successor proves that it completed.
// The virtual index events.len() selects successful VM completion.
fn threshold(
	engine: &Engine,
	context: &AccumulateCallContext<'_>,
	events: &[String],
	index: usize,
	upper: u64,
) -> Result<u64, String> {
	let (mut lo, mut hi) = (0, upper);
	while lo < hi {
		let mid = lo + (hi - lo) / 2;
		let (actual, _, completed) = probe(engine, context, mid)?;
		if !events.starts_with(&actual) {
			return Err("gas-dependent host-call order differs from calibration".into());
		}
		if (index == events.len() && completed) || actual.len() > index {
			hi = mid;
		} else {
			lo = mid + 1;
		}
	}
	Ok(lo)
}

pub fn enabled(frame: &Value) -> bool {
	["replayInvocationGas", "replayGasSample", "replayGasBoundary"]
		.iter()
		.any(|key| frame.get(key).is_some())
}

pub fn limit(
	engine: &Engine,
	context: &AccumulateCallContext<'_>,
	frame: &Value,
) -> Result<u64, String> {
	let absolute = frame.get("replayInvocationGas");
	let sample = frame.get("replayGasSample");
	let boundary = frame.get("replayGasBoundary");
	if [absolute, sample, boundary].iter().filter(|v| v.is_some()).count() > 1 {
		return Err("choose one invocation gas limit, sample, or boundary".into());
	}
	let before_checkpoint = gas::before_checkpoint(frame)?;
	let results = field(frame, "lastStepWorkResults")?.as_array().ok_or("missing work results")?;
	let after_checkpoint = gas::interrupted(frame)? == Some(results.len());
	if !before_checkpoint &&
		!after_checkpoint &&
		!((frame.get("replayStorageBudget").is_some() ||
			(frame.get("replayCodeFailure").is_some() &&
				frame.get("replayHostTrap") == Some(&Value::Bool(false)))) &&
			gas::interrupted(frame)?.is_some()) &&
		absolute.is_none() &&
		sample.is_none() &&
		boundary.is_none()
	{
		return Ok(context.gas);
	}
	if frame.get("replayPanic") == Some(&Value::Bool(true)) {
		return Err("instruction gas and panic injection cannot be combined".into());
	}
	if let Some(value) = absolute {
		let limit = bounded_integer::<u64>(value, "invocation gas")?;
		if limit > i64::MAX as u64 {
			return Err("invocation gas exceeds signed VM range".into());
		}
		return Ok(limit);
	}
	let stop = if before_checkpoint {
		0
	} else {
		gas::interrupted(frame)?.ok_or("instruction gas sampling requires replayInterrupt")?
	};
	let limits = gas::limits(frame, results.len())?;
	// Sampling currently supports valid, admitted reports. Rejected reports
	// remain covered by the ordinary report-budget campaigns.
	if results.iter().zip(limits).any(|(r, l)| {
		variant(&r["result"]).map(|(tag, _)| tag != "WorkOk").unwrap_or(true) || l != u64::MAX
	}) {
		return Err("instruction gas sampling requires admitted WorkOk reports".into());
	}
	// A pool below the declared gas leaves no always-accumulate allowance, so no
	// due assign is flushed (§5.1). Calibrate in that regime as well, one gas
	// short of the service's flush base plus one assign, so the host-call order
	// matches every probe below.
	let declared = context.items.iter().fold(0u64, |sum, item| {
		sum.saturating_add(match item {
			AccumulateItem::WorkItem(record) => record.gas_limit,
			AccumulateItem::Transfer(transfer) => transfer.gas_limit,
		})
	});
	let calibration = context.gas.min(declared.saturating_add(5_000_000 + 1_000_000 - 1));
	let (events, used, completed) = probe(engine, context, calibration)?;
	if !completed {
		return Err("instruction gas calibration exhausted the default pool".into());
	}
	let checkpoints: Vec<_> = events
		.iter()
		.enumerate()
		.filter_map(|(i, e)| (e == "checkpoint").then_some(i))
		.collect();
	if checkpoints.len() != results.len() + 1 {
		return Err(format!(
			"checkpoint count differs: expected {}, actual {}",
			results.len() + 1,
			checkpoints.len()
		));
	}
	let start = if before_checkpoint { 0 } else { checkpoints[stop] + 1 };
	let end = if before_checkpoint {
		checkpoints[0]
	} else if after_checkpoint {
		events.len()
	} else {
		checkpoints[stop + 1]
	};
	// The pinned checkpoint host call has only its fixed charge before taking
	// the snapshot. Paying it proves completion even when no host calls follow
	// (e.g. an empty invocation or unchanged heads). No recovered state is read.
	let final_start = if after_checkpoint {
		Some(
			threshold(engine, context, &events, checkpoints[stop], used)?
				.checked_add(jam_types::gas_costs::CHECKPOINT)
				.ok_or("checkpoint gas overflow")?,
		)
	} else {
		None
	};
	let index = if let Some(boundary) = boundary {
		let name = boundary.as_str().ok_or("gas boundary must be a string")?;
		let (side, call) =
			name.split_once(':').ok_or("gas boundary must be before:call or after:call")?;
		if !matches!(
			call.split(':').next().unwrap(),
			"write" | "new" | "transfer" | "assign" | "read"
		) {
			return Err("unsupported gas boundary call".into());
		}
		// Optional zero-based occurrence targets every write in a multi-write
		// operation, rather than repeatedly stopping at the head-data write.
		let (call, occurrence) = match call.split_once(':') {
			Some((call, occurrence)) => (
				call,
				occurrence
					.parse::<usize>()
					.map_err(|_| "gas boundary occurrence must be an unsigned integer")?,
			),
			None => (call, 0),
		};
		let index =
			(start..end).filter(|&i| events[i] == call).nth(occurrence).ok_or_else(|| {
				format!("gas boundary call absent from selected phase: {:?}", &events[start..end])
			})?;
		match side {
			"before" => Some((index, false)),
			// At a following checkpoint, reaching the attempt pays none of
			// its fixed charge, so its snapshot has not replaced our prefix.
			"after" if index + 1 <= end && index + 1 < events.len() => Some((index + 1, true)),
			_ => return Err("unsupported gas boundary side".into()),
		}
	} else {
		None
	};
	let limit = if let Some((index, after)) = index {
		let threshold = threshold(engine, context, &events, index, used)?;
		if after {
			threshold
		} else {
			threshold.checked_sub(1).ok_or("empty gas boundary")?
		}
	} else {
		let sample = sample
			.map(|v| bounded_integer::<u32>(v, "gas sample"))
			.transpose()?
			.unwrap_or(500_000);
		if sample > 1_000_000 {
			return Err("gas sample must be between 0 and 1000000".into());
		}
		let lo = if before_checkpoint {
			0
		} else if let Some(lo) = final_start {
			lo
		} else {
			threshold(engine, context, &events, start, used)?
		};
		let hi = threshold(engine, context, &events, end, used)?
			.checked_sub(1)
			.ok_or("empty phase gas interval")?;
		if lo > hi {
			return Err("empty phase gas interval".into());
		}
		lo + ((u128::from(hi - lo) * u128::from(sample)) / 1_000_000) as u64
	};
	// This validation depends only on execution order. Never accept a different
	// checkpoint because its recovered state happens to match the oracle.
	let (actual, _, completed) = probe(engine, context, limit)?;
	if completed ||
		(!before_checkpoint && !after_checkpoint && actual.len() <= start) ||
		final_start.is_some_and(|lo| limit < lo) ||
		actual.len() > end + usize::from(index == Some((end, true))) ||
		!events.starts_with(&actual)
	{
		return Err(format!(
			"gas {limit} did not interrupt the predetermined phase (before_checkpoint={before_checkpoint}, report={stop}, after_checkpoint={after_checkpoint})"
		));
	}
	Ok(limit)
}
