//! Locate instruction cutoffs using host-call order, never recovered state.
//!
//! Quint has already chosen the recovery phase and report prefix. A successful,
//! discarded run and gas-only probes locate its interval in the production VM.
//! The final run must still match every Quint state/effect assertion.
use std::{cell::RefCell, sync::OnceLock};

use jam_codec::{Decode, Encode};
use jam_node::vm::{AccumulateCallContext, Engine};
use serde_json::Value;

use super::{gas, replay::*};

thread_local! {
	static EVENTS: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

struct Observer;
impl log::Log for Observer {
	fn enabled(&self, metadata: &log::Metadata) -> bool {
		metadata.target() == "service" && EVENTS.with(|events| events.borrow().is_some())
	}
	fn log(&self, record: &log::Record) {
		if !self.enabled(record.metadata()) {
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

fn observe() -> Result<(), String> {
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

// Minimum gas which reaches the selected host-call attempt. The call itself
// may run out of gas; reaching its successor proves that it completed.
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
		let (actual, _, _) = probe(engine, context, mid)?;
		if !events.starts_with(&actual) {
			return Err("gas-dependent host-call order differs from calibration".into());
		}
		if actual.len() > index {
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
	if !before_checkpoint && absolute.is_none() && sample.is_none() && boundary.is_none() {
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
	let results = field(frame, "lastStepWorkResults")?.as_array().ok_or("missing work results")?;
	let limits = gas::limits(frame, results.len())?;
	// Sampling currently supports valid, admitted reports. Rejected reports
	// remain covered by the ordinary report-budget campaigns.
	if results.iter().zip(limits).any(|(r, l)| {
		variant(&r["result"]).map(|(tag, _)| tag != "WorkOk").unwrap_or(true) || l != u64::MAX
	}) {
		return Err("instruction gas sampling requires admitted WorkOk reports".into());
	}
	let (events, used, completed) = probe(engine, context, context.gas)?;
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
	let end = if before_checkpoint { checkpoints[0] } else { checkpoints[stop + 1] };
	let index = if let Some(boundary) = boundary {
		let name = boundary.as_str().ok_or("gas boundary must be a string")?;
		let (side, call) =
			name.split_once(':').ok_or("gas boundary must be before:call or after:call")?;
		if !matches!(call, "write" | "new" | "transfer" | "assign") {
			return Err("unsupported gas boundary call".into());
		}
		let index = (start..end)
			.find(|&i| events[i] == call)
			.ok_or("gas boundary call absent from selected phase")?;
		match side {
			"before" => Some((index, false)),
			"after" if index + 1 < end => Some((index + 1, true)),
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
		let lo =
			if before_checkpoint { 0 } else { threshold(engine, context, &events, start, used)? };
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
		(!before_checkpoint && actual.len() <= start) ||
		actual.len() > end ||
		!events.starts_with(&actual)
	{
		return Err(format!(
			"gas {limit} did not interrupt the predetermined phase (before_checkpoint={before_checkpoint}, report={stop})"
		));
	}
	Ok(limit)
}
