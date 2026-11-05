//! Successful host calls, preserving effects hidden by final mutations.
//! The hazmat host emits only completed effects and completed checkpoints.
use std::cell::RefCell;

use jam_codec::Encode;
use serde_json::Value;

use super::{assignments, replay::*};

#[derive(Default)]
struct History {
	calls: Vec<String>,
	checkpoint: usize,
}
thread_local! {
	static HISTORY: RefCell<Option<History>> = const { RefCell::new(None) };
}

pub fn enabled() -> bool {
	HISTORY.with(|history| history.borrow().is_some())
}

pub fn record(event: &str) {
	HISTORY.with(|history| {
		if let Some(history) = history.borrow_mut().as_mut() {
			if event == "checkpoint" {
				history.checkpoint = history.calls.len();
			} else if ["assign ", "designate ", "upgrade "]
				.iter()
				.any(|prefix| event.starts_with(prefix))
			{
				history.calls.push(event.to_owned());
			}
		}
	});
}

pub struct Capture;
impl Capture {
	pub fn start() -> Result<Self, String> {
		super::instruction_gas::observe()?;
		HISTORY.with(|history| {
			let mut history = history.borrow_mut();
			if history.is_some() {
				return Err("nested host-call observation".into());
			}
			*history = Some(History::default());
			Ok(Self)
		})
	}

	pub fn compare(&self, frame: &Value, rollback: bool) -> Result<(), String> {
		let expected = field(frame, "lastStepAssigns")?
			.as_array()
			.ok_or("lastStepAssigns must be a list")?
			.iter()
			.map(|call| {
				let core = bounded_integer::<u16>(field(call, "core")?, "assignment core")?;
				let owner = assignments::service_id(field(call, "assigner")?)?;
				let queue = assignments::queue(field(call, "queue")?)?;
				// AuthQueue has fixed length and no vector prefix in JAM encoding.
				let bytes: Vec<u8> = queue.iter().flat_map(Encode::encode).collect();
				Ok(format!("assign {core} {owner} {bytes:?}"))
			})
			.collect::<Result<Vec<_>, String>>()?;
		HISTORY.with(|history| {
			let history = history.borrow();
			let history = history.as_ref().ok_or("missing assignment observation")?;
			let retained = if rollback { history.checkpoint } else { history.calls.len() };
			let assignments: Vec<_> =
				history.calls[..retained].iter().filter(|c| c.starts_with("assign ")).collect();
			if assignments != expected.iter().collect::<Vec<_>>() {
				return Err(format!(
					"ordered assignment calls differ: expected {} calls, observed {retained}",
					expected.len()
				));
			}
			if let Some(calls) = frame.get("replayHostCalls") {
				let expected = expected_calls(calls)?;
				let actual: Vec<_> = history.calls[..retained]
					.iter()
					.filter(|c| !c.starts_with("assign "))
					.collect();
				if actual != expected.iter().collect::<Vec<_>>() {
					return Err(format!(
						"ordered replayHostCalls differ: expected {} calls, observed {}",
						expected.len(),
						actual.len()
					));
				}
			}
			Ok(())
		})
	}
}
impl Drop for Capture {
	fn drop(&mut self) {
		HISTORY.with(|history| *history.borrow_mut() = None);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn checkpoint_history_works() {
		let capture = Capture::start().unwrap();
		record("checkpoint");
		record("assign 1 0 []");
		let empty = json!({"lastStepAssigns": []});
		capture.compare(&empty, true).unwrap();
		assert!(capture.compare(&empty, false).is_err());
		record("checkpoint");
		assert!(capture.compare(&empty, true).is_err());
	}
}

fn expected_calls(value: &Value) -> Result<Vec<String>, String> {
	value
		.as_array()
		.ok_or("replayHostCalls must be a list")?
		.iter()
		.map(|call| {
			let (tag, args) = variant(call)?;
			match tag {
				"DesignateCall" => {
					let bytes: Vec<u8> =
						super::validator_keys::keys(args)?.into_iter().flatten().collect();
					Ok(format!("designate {bytes:?}"))
				},
				"UpgradeCall" => {
					let code = integer(field(field(args, "codeHash")?, "hashBytes")?)?;
					if !super::service_upgrade::is_code(code) {
						return Err("unsupported observed upgrade code".into());
					}
					let hash = jam_std_common::hash_raw(&super::service_upgrade::blob(
						code,
						super::service_upgrade::CODE_LEN,
					)?);
					let acc = bounded_integer::<u64>(field(args, "minAccGas")?, "upgrade gas")?;
					let memo =
						bounded_integer::<u64>(field(args, "minMemoGas")?, "upgrade memo gas")?;
					Ok(format!("upgrade {acc} {memo} {hash:?}"))
				},
				_ => Err(format!("unsupported replayHostCalls event {tag}")),
			}
		})
		.collect()
}

pub fn validate(document: &Value, states: &[Value]) -> Result<(), String> {
	let present = states.first().is_some_and(|s| s.get("replayHostCalls").is_some());
	let required = document
		.get("replayHostCallsRequired")
		.map(boolean)
		.transpose()?
		.unwrap_or(false);
	if required && !present {
		return Err("replayHostCalls required by this trace".into());
	}
	for (index, state) in states.iter().enumerate() {
		if state.get("replayHostCalls").is_some() != present {
			return Err("replayHostCalls must be present in every frame".into());
		}
		if let Some(calls) = state.get("replayHostCalls") {
			let calls = calls.as_array().ok_or("replayHostCalls must be a list")?;
			if index == 0 && !calls.is_empty() {
				return Err("initial replayHostCalls must be empty".into());
			}
		}
	}
	Ok(())
}
