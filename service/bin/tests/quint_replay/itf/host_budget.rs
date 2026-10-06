//! Temporary JAM balance inputs for mixed invocations. Only the balance is
//! changed; write failures and checkpoint rollback execute in the real PVM.
use jam_node::vm::Storage;
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use serde_json::Value;

use super::replay::*;

pub fn validate(states: &[Value]) -> Result<(), String> {
	let enabled = states.first().is_some_and(|s| s.get("replayHostBudget").is_some());
	for state in states {
		for key in ["replayHostBudget", "replayHostFree", "replayHostRejects"] {
			if state.get(key).is_some() != enabled {
				return Err("mixed host budget requires all three fields in every frame".into());
			}
		}
		if !enabled {
			continue;
		}
		let budget = integer(field(state, "replayHostBudget")?)?;
		let free = integer(field(state, "replayHostFree")?)?;
		let rejects =
			bounded_integer::<u64>(field(state, "replayHostRejects")?, "host rejections")?;
		if budget == -1 {
			if free != -1 || rejects != 0 {
				return Err("inactive host budget has outcomes".into());
			}
		} else {
			bounded_integer::<u64>(field(state, "replayHostBudget")?, "host budget")?;
			bounded_integer::<u64>(field(state, "replayHostFree")?, "host free balance")?;
			if state.get("replayStorageBudget").is_some() {
				return Err("mixed and isolated storage budgets cannot be combined".into());
			}
		}
	}
	if enabled && integer(field(&states[0], "replayHostBudget")?)? != -1 {
		return Err("initial mixed host budget must be inactive".into());
	}
	Ok(())
}

pub fn prepare(storage: &mut Storage, frame: &Value) -> Result<Option<i128>, String> {
	let Some(value) = frame.get("replayHostBudget") else { return Ok(None) };
	if integer(value)? == -1 {
		return Ok(None);
	}
	if jam_types::deposit_per_item() != 10 ||
		jam_types::deposit_per_byte() != 1 ||
		jam_types::FOOTPRINT_STORAGE_OVERHEAD != 34 ||
		jam_types::FOOTPRINT_PREIMAGE_OVERHEAD != 81
	{
		return Err("mixed host budget uses the pinned JAM deposit parameters".into());
	}
	let allowance = bounded_integer::<u64>(value, "host budget")?;
	let mut service = storage.service(MOCK_SERVICE_ID).ok_or("missing host-budget service")?;
	let balance = service.threshold().checked_add(allowance).ok_or("host budget overflow")?;
	let offset = i128::from(service.balance) - i128::from(balance);
	service.balance = balance;
	storage.set_service(MOCK_SERVICE_ID, &service);
	storage.commit();
	Ok(Some(offset))
}

pub fn finish(
	storage: &mut Storage,
	frame: &Value,
	offset: Option<i128>,
	index: usize,
) -> Result<(), String> {
	let Some(offset) = offset else { return Ok(()) };
	let mut service = storage.service(MOCK_SERVICE_ID).ok_or("missing host-budget service")?;
	let expected = bounded_integer::<u64>(field(frame, "replayHostFree")?, "host free balance")?;
	if service.balance < service.threshold() || service.free() != expected {
		return Err(format!(
			"frame {index}: mixed host free balance differs: expected {expected}, actual {}",
			service.free()
		));
	}
	// Restore only the injected offset, retaining actual incoming credits and
	// outgoing debits. Later ordinary invocations continue from the same state.
	service.balance = u64::try_from(i128::from(service.balance) + offset)
		.map_err(|_| "restored host balance out of range")?;
	storage.set_service(MOCK_SERVICE_ID, &service);
	storage.commit();
	Ok(())
}
