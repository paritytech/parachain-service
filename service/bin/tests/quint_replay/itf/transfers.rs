//! Literal incoming operands and the model's bucket layout. Never reconstruct
//! operands from the expected queue: that would hide incorrect admission/drop.
use jam_node::vm::Storage;
use jam_types::{AccumulateItem, Memo, TransferRecord};
use parachain_service::state::{
	storage_key,
	transfers::{IncomingTransferBuckets, IncomingTransfers, QueuedTransfer},
	Tag,
};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use serde_json::Value;

use super::replay::*;
use crate::common::get_state;

fn transfer(value: &Value) -> Result<QueuedTransfer, String> {
	let (tag, source) = variant(field(value, "from")?)?;
	if tag != "MkServiceId" {
		return Err("expected incoming MkServiceId".into());
	}
	// Abstract memo integers map injectively to a zero-padded 128-byte memo.
	let n = bounded_integer::<u64>(field(value, "memo")?, "incoming memo")?;
	let mut memo = [0; 128];
	memo[..8].copy_from_slice(&n.to_le_bytes());
	if boolean(field(value, "toSupervisorBalance")?)? {
		return Err("supervisor-balance incoming transfers are unsupported by the JAM host".into());
	}
	Ok(QueuedTransfer {
		from: bounded_integer::<u32>(source, "incoming source")?,
		amount: bounded_integer::<u64>(field(value, "amount")?, "incoming amount")?,
		to_supervisor_balance: false,
		memo,
	})
}

pub fn operands(frame: &Value) -> Result<Vec<AccumulateItem>, String> {
	field(frame, "replayIncoming")?
		.as_array()
		.ok_or("replayIncoming must be a list")?
		.iter()
		.map(|value| {
			let t = transfer(value)?;
			Ok(AccumulateItem::Transfer(TransferRecord {
				source: t.from,
				destination: MOCK_SERVICE_ID,
				amount: t.amount,
				memo: Memo(t.memo),
				gas_limit: 1_000_000,
			}))
		})
		.collect()
}

/// Return the exact expected keys so the caller also rejects orphan buckets.
pub fn compare(storage: &Storage, svc: &Value, frame: usize) -> Result<Vec<Vec<u8>>, String> {
	let endpoints = svc
		.get("incomingTransferBuckets")
		.or_else(|| svc.get("incomingTransferChain"))
		.ok_or("missing incoming transfer endpoints")?;
	let expected = match variant(endpoints)? {
		("None", _) => None,
		("Some", value) => Some(IncomingTransferBuckets {
			first_bucket: bounded_integer::<u64>(field(value, "firstBucket")?, "first bucket")?,
			last_bucket: bounded_integer::<u64>(field(value, "lastBucket")?, "last bucket")?,
			count: bounded_integer::<u32>(field(value, "count")?, "transfer count")?,
		}),
		_ => return Err("invalid incoming transfer endpoints".into()),
	};
	let key = storage_key(Tag::IncomingTransferBuckets, &());
	if (expected.is_none() && storage.service_key(MOCK_SERVICE_ID, &key).is_some()) ||
		get_state::<IncomingTransferBuckets>(storage, &key) != expected
	{
		return Err(format!("frame {frame}: svc.incomingTransferBuckets differs"));
	}
	let mut keys = if expected.is_some() { vec![key] } else { Vec::new() };
	let mut ids = std::collections::BTreeSet::new();
	for (id, bucket) in map_entries(field(svc, "incomingTransfers")?)? {
		let id = bounded_integer::<u64>(id, "incoming bucket id")?;
		if !ids.insert(id) {
			return Err("duplicate incoming bucket id".into());
		}
		let expected: IncomingTransfers = bucket
			.as_array()
			.ok_or("incoming bucket must be a list")?
			.iter()
			.map(transfer)
			.collect::<Result<Vec<_>, _>>()?
			.try_into()
			.map_err(|_| "incoming bucket exceeds capacity")?;
		let key = storage_key(Tag::IncomingTransfers, &id);
		if get_state::<IncomingTransfers>(storage, &key) != Some(expected) {
			return Err(format!("frame {frame}: svc.incomingTransfers[{id}] differs"));
		}
		keys.push(key);
	}
	Ok(keys)
}
