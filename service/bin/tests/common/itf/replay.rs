use codec::Compact;
use jam_node::vm::Storage;
use jam_std_common::hash_raw;
use parachain_service::work_digest::ParachainWorkDigest;
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_interface::{types::ParaId, upward_message::UpwardMessage};
use serde_json::Value;

use super::{
	classify::{classify, FrameKind},
	codex::Codex,
	compare, seed,
};
use crate::common::{accumulate_block, fresh_storage, work_item};

/// Replay a normalized Quint trace. Blocks are deliberately limited to one WP.
pub fn trace(json: &str) -> Result<(), String> {
	let document: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
	// Validate every Quint value strictly before using the ergonomic JSON view.
	super::value::ItfValue::try_from(&document)?;
	let states = document.get("states").and_then(Value::as_array).ok_or("missing states")?;
	let first = states.first().ok_or("trace has no states")?;
	let mut codex = Codex::default();
	let mut storage = fresh_storage(|storage| seed::seed(storage, first, &mut codex).unwrap());
	compare::state(&storage, first, &mut codex, 0)?;

	for (index, pair) in states.windows(2).enumerate() {
		let frame = index + 1;
		match classify(&pair[0], &pair[1])? {
			FrameKind::Noop => continue,
			FrameKind::Block => {
				let results = field(&pair[1], "lastStepWorkResults")?
					.as_array()
					.ok_or("lastStepWorkResults must be a list")?;
				if results.len() > 1 {
					return Err(format!(
						"frame {frame}: one-WP replay does not support {} work results",
						results.len()
					));
				}
				let items = results
					.first()
					.map(|result| {
						work_result(result, &mut codex).map(|digest| vec![work_item(&digest)])
					})
					.transpose()?
					.unwrap_or_default();
				let slot = integer(field(&pair[1], "now")?)? as u32;
				let (_, next, _) = accumulate_block(storage, items, slot);
				storage = next;
			},
			FrameKind::ProvisionPreimage => {
				provision(&mut storage, &pair[0], &pair[1], &mut codex)?
			},
			FrameKind::IncomingTransfer => {
				return Err(format!("frame {frame}: incoming-transfer replay is not implemented"))
			},
		}
		compare::state(&storage, &pair[1], &mut codex, frame)?;
	}
	Ok(())
}

fn work_result(value: &Value, codex: &mut Codex) -> Result<ParachainWorkDigest, String> {
	let (tag, value) = variant(field(value, "result")?)?;
	if tag != "WorkOk" {
		return Err(format!("unsupported work result {tag}"));
	}
	let (tag, digest) = variant(value)?;
	if tag != "Ok" {
		return Err(format!("unsupported refine result {tag}"));
	}
	let para = para_id(field(digest, "paraId")?, codex)?;
	let validation = field(digest, "validationCode")?;
	let validation_code = codex.validation_code(
		integer(field(field(validation, "hash")?, "vchBytes")?)?,
		integer(field(validation, "len")?)?,
	)?;
	let parent = Codex::head(integer(field(field(digest, "parentHeadHash")?, "headBytes")?)?)?;
	let messages = field(digest, "upwardMessages")?
		.as_array()
		.ok_or("upwardMessages must be a list")?
		.iter()
		.map(|message| upward_message(message, codex))
		.collect::<Result<Vec<_>, _>>()?;
	Ok(ParachainWorkDigest::Ok {
		para_id: para,
		validation_code,
		parent_head_hash: hash_raw(&parent),
		head_data: Codex::head(integer(field(digest, "headData")?)?)?,
		upward_messages: messages.try_into().map_err(|_| "too many upward messages")?,
		lookup_anchor: integer(field(digest, "lookupAnchor")?)? as u32,
	})
}

fn upward_message(value: &Value, codex: &mut Codex) -> Result<UpwardMessage, String> {
	let (tag, value) = variant(value)?;
	match tag {
		"RequestCodeUpgrade" => {
			let len = integer(field(value, "len")?)?;
			let reference =
				codex.validation_code(integer(field(field(value, "hash")?, "vchBytes")?)?, len)?;
			Ok(UpwardMessage::RequestCodeUpgrade {
				hash: reference.hash,
				len: Compact(reference.len),
			})
		},
		"ParachainSetStateBalance" => Ok(UpwardMessage::ParachainSetStateBalance {
			para_id: para_id(field(value, "paraId")?, codex)?,
			new_total: Compact(integer(field(value, "newTotal")?)? as u64),
		}),
		"ParachainSetValidationCode" => {
			let len = integer(field(value, "newValidationCodeLen")?)?;
			let reference = codex.validation_code(
				integer(field(field(value, "newValidationCodeHash")?, "vchBytes")?)?,
				len,
			)?;
			Ok(UpwardMessage::ParachainSetValidationCode {
				para_id: para_id(field(value, "paraId")?, codex)?,
				new_validation_code_hash: reference.hash,
				new_validation_code_len: Compact(reference.len),
			})
		},
		other => Err(format!("unsupported upward message {other}")),
	}
}

fn provision(
	storage: &mut Storage,
	previous: &Value,
	current: &Value,
	codex: &mut Codex,
) -> Result<(), String> {
	let before = map_entries(field(field(previous, "svc")?, "preimageStatus")?)?;
	for (key, status) in map_entries(field(field(current, "svc")?, "preimageStatus")?)? {
		if variant(status)?.0 != "Provided" ||
			before.iter().any(|(old_key, old_status)| {
				*old_key == key && variant(old_status).map(|v| v.0 == "Provided").unwrap_or(false)
			}) {
			continue;
		}
		let key = tuple(key)?;
		let abstract_hash = integer(field(&key[0], "hashBytes")?)?;
		let len = integer(&key[1])? as u32;
		let blob = Codex::blob(abstract_hash, len)?;
		let expected_hash = codex.hash(abstract_hash, len)?;
		if hash_raw(&blob) != expected_hash {
			return Err("codex preimage hash mismatch".into());
		}
		storage
			.provide(integer(field(current, "now")?)? as u32, MOCK_SERVICE_ID, &blob)
			.map_err(|_| "host rejected provisioned preimage")?;
		storage.commit();
		debug_assert!(storage
			.lookup_request(MOCK_SERVICE_ID, expected_hash, len)
			.is_some_and(|request| request.is_available()));
		return Ok(());
	}
	Err("provision frame did not make a preimage Provided".into())
}

pub(crate) fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, String> {
	value.get(name).ok_or_else(|| format!("missing field {name}"))
}
pub(crate) fn integer(value: &Value) -> Result<i128, String> {
	if let Some(value) = value.get("#bigint").and_then(Value::as_str) {
		return value.parse().map_err(|_| "invalid #bigint".into());
	}
	// Quint's JSON writer sometimes leaks BigNumber's internal representation
	// for large integers: sign, decimal exponent, and base-1e14 coefficient limbs.
	let sign = integer(value.get("s").ok_or("expected #bigint")?)?;
	let exponent = integer(value.get("e").ok_or("invalid BigNumber exponent")?)?;
	let limbs = value.get("c").and_then(Value::as_array).ok_or("invalid BigNumber limbs")?;
	let mut digits = String::new();
	for (index, limb) in limbs.iter().enumerate() {
		let limb = u64::try_from(integer(limb)?).map_err(|_| "invalid BigNumber limb")?;
		if index == 0 {
			digits.push_str(&limb.to_string())
		} else {
			digits.push_str(&format!("{limb:014}"))
		}
	}
	let integer_digits =
		usize::try_from(exponent + 1).map_err(|_| "BigNumber is not an integer")?;
	if integer_digits < digits.len() && digits[integer_digits..].bytes().any(|digit| digit != b'0')
	{
		return Err("BigNumber has a fractional part".into());
	}
	digits.truncate(integer_digits.min(digits.len()));
	digits.extend(core::iter::repeat_n('0', integer_digits.saturating_sub(digits.len())));
	let magnitude: i128 = digits.parse().map_err(|_| "BigNumber out of i128 range")?;
	Ok(if sign < 0 { -magnitude } else { magnitude })
}
pub(crate) fn boolean(value: &Value) -> Result<bool, String> {
	value.as_bool().ok_or("expected bool".into())
}
pub(crate) fn variant(value: &Value) -> Result<(&str, &Value), String> {
	Ok((field(value, "tag")?.as_str().ok_or("variant tag must be string")?, field(value, "value")?))
}
pub(crate) fn tuple(value: &Value) -> Result<&Vec<Value>, String> {
	field(value, "#tup")?.as_array().ok_or("expected #tup".into())
}
pub(crate) fn set_values(value: &Value) -> Result<&Vec<Value>, String> {
	field(value, "#set")?.as_array().ok_or("expected #set".into())
}
pub(crate) fn map_entries(value: &Value) -> Result<Vec<(&Value, &Value)>, String> {
	field(value, "#map")?
		.as_array()
		.ok_or("expected #map")?
		.iter()
		.map(|entry| {
			let pair = entry.as_array().ok_or("map entry must be pair")?;
			if pair.len() != 2 {
				return Err("map entry must have two values".into());
			}
			Ok((&pair[0], &pair[1]))
		})
		.collect()
}
pub(crate) fn para_id(value: &Value, _codex: &Codex) -> Result<ParaId, String> {
	let (tag, value) = variant(value)?;
	if tag != "MkParaId" {
		return Err(format!("expected MkParaId, got {tag}"));
	}
	Codex::para_id(integer(value)?)
}
