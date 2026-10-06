//! TransferOut operands, ordered JAM effects, and regular-balance accounting.
use jam_node::vm::{StateMutations, Storage};
use jam_types::{Memo, TransferRecord};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_core::upward_message::{TransferOutArgs, UpwardMessage};
use serde_json::Value;

use super::{assignments::service_id, replay::*};

fn memo(value: &Value) -> Result<[u8; 128], String> {
	let mut bytes = [0; 128];
	bytes[..8].copy_from_slice(&bounded_integer::<u64>(value, "outgoing memo")?.to_le_bytes());
	Ok(bytes)
}

pub fn message(value: &Value) -> Result<UpwardMessage, String> {
	let source = match variant(field(value, "source")?)? {
		("None", _) => None,
		("Some", source) => Some(service_id(source)?),
		_ => return Err("invalid outgoing source option".into()),
	};
	let deferred = match variant(field(value, "deferred")?)? {
		("None", _) => None,
		("Some", pair) => {
			let pair = tuple(pair)?;
			if pair.len() != 2 {
				return Err("deferred transfer must contain memo and gas".into());
			}
			Some((memo(&pair[0])?, bounded_integer::<u64>(&pair[1], "transfer gas")?))
		},
		_ => return Err("invalid deferred transfer option".into()),
	};
	Ok(UpwardMessage::TransferOut(TransferOutArgs {
		source,
		dest: service_id(field(value, "dest")?)?,
		amount: bounded_integer::<u64>(field(value, "amount")?, "transfer amount")?.into(),
		id: bounded_integer::<u64>(field(value, "id")?, "transfer id")?.into(),
		source_supervisor_balance: boolean(field(value, "sourceSupervisorBalance")?)?,
		dest_supervisor_balance: boolean(field(value, "destSupervisorBalance")?)?,
		deferred,
	}))
}

fn account(value: &Value) -> Result<(u64, u64), String> {
	if bounded_integer::<u64>(field(value, "supervisorBalance")?, "supervisor balance")? != 0 {
		return Err("funded supervisor balances are unsupported by the JAM host".into());
	}
	Ok((
		bounded_integer(field(value, "balance")?, "JAM balance")?,
		bounded_integer(field(value, "minMemoGas")?, "minimum memo gas")?,
	))
}

/// Opt in only for traces carrying the transfer oracle; historical fixtures
/// intentionally do not model real JAM balances.
pub fn seed(storage: &mut Storage, frame: &Value) -> Result<(), String> {
	if frame.get("replayTransfers").is_none() {
		return Ok(());
	}
	let (balance, min_memo_gas) = account(field(field(frame, "svc")?, "jamAccount")?)?;
	let mut own = storage.service(MOCK_SERVICE_ID).ok_or("missing own service")?;
	own.balance = balance;
	own.min_memo_gas = min_memo_gas;
	storage.set_service(MOCK_SERVICE_ID, &own);
	for (id, foreign) in map_entries(field(frame, "foreignServices")?)? {
		let id = service_id(id)?;
		if id == MOCK_SERVICE_ID ||
			service_id(field(foreign, "supervisor")?)? != id ||
			!map_entries(field(foreign, "requests")?)?.is_empty() ||
			!map_entries(field(foreign, "storage")?)?.is_empty()
		{
			return Err("outgoing replay requires empty, self-supervised foreign services".into());
		}
		let (balance, min_memo_gas) = account(field(foreign, "account")?)?;
		crate::common::seed_service(storage, id, min_memo_gas);
		let mut service = storage.service(id).ok_or("missing foreign service")?;
		service.balance = balance;
		storage.set_service(id, &service);
	}
	Ok(())
}

pub fn compare(current: &Value, mutations: &StateMutations, frame: usize) -> Result<(), String> {
	let mut expected = Vec::new();
	if let Some(transfers) = current.get("replayTransfers") {
		for value in transfers.as_array().ok_or("replayTransfers must be a list")? {
			let UpwardMessage::TransferOut(args) = message(value)? else { unreachable!() };
			if args.source.is_some_and(|id| id != MOCK_SERVICE_ID) ||
				(args.amount.0 != 0 &&
					(args.source_supervisor_balance || args.dest_supervisor_balance))
			{
				return Err("successful supervised transfer is unsupported by the JAM host".into());
			}
			let (memo, gas_limit) =
				args.deferred.ok_or("successful plain move is unsupported by the JAM host")?;
			expected.push(TransferRecord {
				source: MOCK_SERVICE_ID,
				destination: args.dest,
				amount: args.amount.0,
				memo: Memo(memo),
				gas_limit,
			});
		}
	}
	if mutations.transfers.len() != expected.len() ||
		mutations.transfers.iter().zip(&expected).any(|(a, b)| {
			a.source != b.source ||
				a.destination != b.destination ||
				a.amount != b.amount ||
				a.memo.0 != b.memo.0 ||
				a.gas_limit != b.gas_limit
		}) {
		return Err(format!(
			"frame {frame}: replayTransfers differs; Quint={expected:?}; Rust={:?}",
			mutations.transfers
		));
	}
	Ok(())
}

/// The VM executes the sender only. Model the scheduler's later credit using
/// actual emitted records, after their complete contents/order were compared.
pub fn credit_outputs(storage: &mut Storage, mutations: &StateMutations) -> Result<(), String> {
	if mutations.transfers.is_empty() {
		return Ok(());
	}
	for t in &mutations.transfers {
		credit(storage, t.destination, t.amount)?;
	}
	storage.commit();
	Ok(())
}

pub fn credit(storage: &mut Storage, id: u32, amount: u64) -> Result<(), String> {
	let mut service = storage.service(id).ok_or("missing credited service")?;
	service.balance = service.balance.checked_add(amount).ok_or("credited balance overflow")?;
	storage.set_service(id, &service);
	Ok(())
}

pub fn balances(storage: &Storage, current: &Value, frame: usize) -> Result<(), String> {
	if current.get("replayTransfers").is_none() {
		return Ok(());
	}
	let mut accounts = vec![(MOCK_SERVICE_ID, field(field(current, "svc")?, "jamAccount")?)];
	for (id, foreign) in map_entries(field(current, "foreignServices")?)? {
		accounts.push((service_id(id)?, field(foreign, "account")?));
	}
	for (id, value) in accounts {
		let (balance, min_memo_gas) = account(value)?;
		let actual = storage.service(id).ok_or("missing compared service")?;
		if actual.balance != balance || actual.min_memo_gas != min_memo_gas {
			return Err(format!(
				"frame {frame}: JAM account {id} differs; expected balance={balance}, actual={}",
				actual.balance
			));
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn fixture() -> Value {
		serde_json::from_str(include_str!(
			"../../fixtures/quint/outgoing/ordered_repeated_ids_works.itf.json"
		))
		.unwrap()
	}
	fn n(value: i128) -> Value {
		json!({"#bigint":value.to_string()})
	}

	#[test]
	fn transfer_contents_and_order_errors() {
		let trace = fixture();
		let frame = &trace["states"][1];
		let mut good = StateMutations::new(0);
		// Explicit expected records, independent of the adapter's decoder.
		for (destination, amount, gas_limit) in
			[(1, 11, 100), (7, 33, 200), (1, 0, 101), (7, 44, 201)]
		{
			let mut memo = [0; 128];
			memo[0] = 7;
			good.transfers.push_back(TransferRecord {
				source: MOCK_SERVICE_ID,
				destination,
				amount,
				gas_limit,
				memo: Memo(memo),
			});
		}
		compare(frame, &good, 1).unwrap();
		for kind in 0..9 {
			let mut bad = good.clone();
			match kind {
				0 => bad.transfers[0].source = 99,
				1 => bad.transfers[0].destination = 99,
				2 => bad.transfers[0].amount += 1,
				3 => bad.transfers[0].gas_limit += 1,
				4 => bad.transfers[0].memo.0[127] = 1,
				5 => {
					bad.transfers.pop_back();
				},
				6 => bad.transfers.push_back(good.transfers[0].clone()),
				7 => bad.transfers.swap(0, 1),
				_ => bad.transfers.clear(),
			}
			assert!(
				compare(frame, &bad, 1).unwrap_err().contains("replayTransfers"),
				"mutation {kind}"
			);
		}
		assert!(compare(&json!({"replayTransfers":[]}), &good, 1).is_err());
		assert!(compare(&json!({}), &good, 1).is_err());
	}

	#[test]
	fn malformed_transfer_values_errors() {
		let original = fixture()["states"][1]["replayTransfers"][0].clone();
		for (path, value) in [
			("/amount", n(-1)),
			("/amount", n(1i128 << 64)),
			("/id", n(-1)),
			("/id", n(1i128 << 64)),
			("/dest/value", n(1i128 << 32)),
			("/dest/tag", json!("MkParaId")),
			("/deferred/value/#tup/0", n(-1)),
			("/deferred/value/#tup/0", n(1i128 << 64)),
			("/deferred/value/#tup/1", n(-1)),
			("/deferred/value/#tup/1", n(1i128 << 64)),
			("/deferred/value/#tup", json!([])),
			("/deferred/tag", json!("Bad")),
			("/source", json!({"tag":"Some","value":{"tag":"MkServiceId","value":n(-1)}})),
			("/source/tag", json!("Bad")),
			("/sourceSupervisorBalance", n(0)),
		] {
			let mut bad = original.clone();
			*bad.pointer_mut(path).unwrap() = value;
			assert!(message(&bad).is_err(), "{path}");
		}
	}

	#[test]
	fn missing_oracle_errors() {
		let mut trace = fixture();
		trace["states"][1].as_object_mut().unwrap().remove("replayTransfers");
		assert!(document_trace(&trace).unwrap_err().contains("require replayTransfers"));
		for frame in trace["states"].as_array_mut().unwrap() {
			frame.as_object_mut().unwrap().remove("replayTransfers");
		}
		assert!(document_trace(&trace).unwrap_err().contains("require replayTransfers"));
	}

	#[test]
	fn mutated_balance_errors() {
		let trace = fixture();
		for path in ["/svc/jamAccount/balance", "/foreignServices/#map/0/1/account/balance"] {
			let mut bad = trace.clone();
			let value = bad["states"][1].pointer_mut(path).unwrap();
			*value = n(integer(value).unwrap() + 1);
			assert!(document_trace(&bad).unwrap_err().contains("JAM account"), "{path}");
		}
	}
}
