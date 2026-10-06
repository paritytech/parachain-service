use codec::Compact;
use jam_node::vm::Storage;
use jam_std_common::hash_raw;
use jam_types::AccumulateItem;
use parachain_service::work_digest::ParachainWorkDigest;
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_core::{
	types::ParaId,
	upward_message::{CodeUpgradePhase, Target, UpwardMessage},
};
use serde_json::Value;

use super::{
	classify::{classify, FrameKind},
	codex::Codex,
	compare,
	refine_log::refine_log,
	seed,
};
use crate::common::{fresh_storage, work_item_skipped, work_item_with_auth_trace};

/// Replay a normalized Quint trace, preserving work-result order within each block.
pub fn trace(json: &str) -> Result<(), String> {
	let document: Value = serde_json::from_str(json).map_err(|error| error.to_string())?;
	document_trace(&document)
}

/// Replay an already parsed trace, avoiding a serialize/parse round trip for streams.
pub fn document_trace(document: &Value) -> Result<(), String> {
	// Validate every Quint value strictly before using the ergonomic JSON view.
	super::value::ItfValue::try_from(document)?;
	let states = document.get("states").and_then(Value::as_array).ok_or("missing states")?;
	let first = states.first().ok_or("trace has no states")?;
	let upgrade_profile = first.get("replayMinAccGas").is_some();
	if states
		.iter()
		.any(|state| state.get("replayMinAccGas").is_some() != upgrade_profile)
	{
		return Err("UpgradeService traces require replayMinAccGas in every frame".into());
	}
	let transfer_profile = first.get("replayTransfers").is_some();
	if states
		.iter()
		.any(|state| state.get("replayTransfers").is_some() != transfer_profile)
	{
		return Err(
			"TransferOut traces require replayTransfers in every frame, including initialization"
				.into(),
		);
	}
	let mut invariant_errors = Vec::new();
	let mut codex = Codex::default();
	let mut seeded = Ok(());
	let mut storage = fresh_storage(|storage| seeded = seed::seed(storage, first, &mut codex));
	seeded?;
	compare::state(&storage, first, &mut codex, 0)?;
	if let Err(error) = super::invariants::state(&storage, first, &mut codex, 0) {
		invariant_errors.push(error);
	}
	super::outgoing::compare(first, &jam_node::vm::StateMutations::new(0), 0)?;
	super::outgoing::balances(&storage, first, 0)?;
	let mut privileges = super::assignments::initial_privileges(storage.clone());

	for (index, pair) in states.windows(2).enumerate() {
		let frame = index + 1;
		let before_heads = super::invariants::heads(&storage, &codex)?;
		let mut output = None;
		match classify(&pair[0], &pair[1])? {
			FrameKind::Noop => {
				if let Err(error) = super::invariants::state(&storage, &pair[1], &mut codex, frame)
				{
					invariant_errors.push(error);
				}
				if let Err(error) =
					super::invariant_heads::unchanged(&before_heads, &storage, &mut codex, frame)
				{
					invariant_errors.push(error);
				}
				continue;
			},
			FrameKind::Block => {
				let results = field(&pair[1], "lastStepWorkResults")?
					.as_array()
					.ok_or("lastStepWorkResults must be a list")?;
				if pair[1].get("replayDesignate").is_none() &&
					results.iter().any(|r| {
						r.pointer("/result/value/value/upwardMessages")
							.and_then(Value::as_array)
							.is_some_and(|msgs| {
								msgs.iter().any(|m| {
									m.get("tag").and_then(Value::as_str) == Some("SetValidatorKeys")
								})
							})
					}) {
					return Err(format!(
						"frame {frame}: validator-key inputs require replayDesignate"
					));
				}
				if pair[1].get("replayTransfers").is_none() &&
					results.iter().any(|r| {
						r.pointer("/result/value/value/upwardMessages")
							.and_then(Value::as_array)
							.is_some_and(|msgs| {
								msgs.iter().any(|m| {
									m.get("tag").and_then(Value::as_str) == Some("TransferOut")
								})
							})
					}) {
					return Err(format!(
						"frame {frame}: TransferOut inputs require replayTransfers"
					));
				}
				if !upgrade_profile &&
					results.iter().any(|r| {
						r.pointer("/result/value/value/upwardMessages")
							.and_then(Value::as_array)
							.is_some_and(|msgs| {
								msgs.iter().any(|m| {
									m.get("tag").and_then(Value::as_str) == Some("UpgradeService")
								})
							})
					}) {
					return Err("UpgradeService inputs require replayMinAccGas".into());
				}
				let items = results
					.iter()
					.map(|result| work_item(result, &mut codex))
					.collect::<Result<Vec<_>, _>>()?;
				let slot = bounded_integer::<u32>(field(&pair[1], "now")?, "now")?;
				let (outcome, next, mutations) =
					accumulate_block(storage, items, slot, privileges.clone())?;
				output = Some((outcome.yielded, mutations));
				storage = next;
			},
			FrameKind::ProvisionPreimage => {
				provision(&mut storage, &pair[0], &pair[1], &mut codex)?
			},
			FrameKind::IncomingTransfer => {
				let items = super::transfers::operands(&pair[1])?;
				if pair[1].get("replayTransfers").is_some() {
					for item in &items {
						if let AccumulateItem::Transfer(t) = item {
							super::outgoing::credit(&mut storage, MOCK_SERVICE_ID, t.amount)?;
						}
					}
					storage.commit();
				}
				let slot = bounded_integer::<u32>(field(&pair[1], "now")?, "now")?;
				let (outcome, next, mutations) =
					accumulate_block(storage, items, slot, privileges.clone())?;
				output = Some((outcome.yielded, mutations));
				storage = next;
			},
		}
		compare::state(&storage, &pair[1], &mut codex, frame)?;
		if let Err(error) = super::invariants::state(&storage, &pair[1], &mut codex, frame) {
			invariant_errors.push(error);
		}
		if let Some((yielded, mutations)) = output {
			if let Err(error) = super::invariants::effects(&mutations, frame) {
				invariant_errors.push(error);
			}
			if let Err(error) = super::invariant_heads::transition(
				&before_heads,
				&storage,
				&pair[1],
				yielded,
				&mut codex,
				frame,
			) {
				invariant_errors.push(error);
			}
			super::assignments::compare(&pair[1], &mutations, &privileges, frame)?;
			if let Some(next) = &mutations.privileges {
				privileges = next.clone();
			}
			super::compare_output::state(
				&pair[0], &pair[1], yielded, &mutations, &mut codex, frame,
			)?;
			super::outgoing::credit_outputs(&mut storage, &mutations)?;
		} else if let Err(error) =
			super::invariant_heads::unchanged(&before_heads, &storage, &mut codex, frame)
		{
			invariant_errors.push(error);
		}
		super::outgoing::balances(&storage, &pair[1], frame)?;
	}
	if invariant_errors.is_empty() {
		Ok(())
	} else {
		Err(invariant_errors.join("\n"))
	}
}

fn work_item(value: &Value, codex: &mut Codex) -> Result<AccumulateItem, String> {
	let auth_trace = Codex::auth_trace(integer(field(value, "authTrace")?)?)?;
	let (tag, value) = variant(field(value, "result")?)?;
	match tag {
		"WorkOk" => Ok(work_item_with_auth_trace(&work_digest(value, codex)?, auth_trace)),
		// Gray paper `WorkExecResult::Error`: JAM substituted an error for this
		// work-item before the service's refine ran, so Accumulate sees the
		// no-op case (§3.3).
		"WorkErr" => Ok(work_item_skipped(auth_trace)),
		_other => Err(format!("unsupported work result {tag}")),
	}
}

fn work_digest(value: &Value, codex: &mut Codex) -> Result<ParachainWorkDigest, String> {
	let (tag, value) = variant(value)?;
	match tag {
		"Ok" => digest_ok(value, codex),
		"Err" => digest_err(value, codex),
		_other => Err(format!("unsupported refine result {tag}")),
	}
}

fn digest_err(value: &Value, codex: &mut Codex) -> Result<ParachainWorkDigest, String> {
	let para = para_id(field(value, "paraId")?, codex)?;
	let validation_code =
		codex.code_hash(integer(field(field(value, "validationCode")?, "vchBytes")?)?)?;
	let error = refine_log(field(value, "error")?)?;
	Ok(ParachainWorkDigest::Err { para_id: para, validation_code, error })
}

fn digest_ok(value: &Value, codex: &mut Codex) -> Result<ParachainWorkDigest, String> {
	let para = para_id(field(value, "paraId")?, codex)?;
	let validation_code =
		codex.code_hash(integer(field(field(value, "validationCode")?, "vchBytes")?)?)?;
	let parent_hash = field(value, "parentHeadHash")?;
	let parent = Codex::head(integer(
		parent_hash
			.get("headBytes")
			.or_else(|| parent_hash.get("hashBytes"))
			.ok_or("missing parent head hash")?,
	)?)?;
	let messages = field(value, "upwardMessages")?
		.as_array()
		.ok_or("upwardMessages must be a list")?
		.iter()
		.map(|message| upward_message(message, para, codex))
		.collect::<Result<Vec<_>, _>>()?;
	Ok(ParachainWorkDigest::Ok {
		para_id: para,
		validation_code,
		parent_head_hash: hash_raw(&parent),
		head_data: Codex::head(integer(field(value, "headData")?)?)?,
		upward_messages: messages.try_into().map_err(|_| "too many upward messages")?,
		lookup_anchor: bounded_integer::<u32>(field(value, "lookupAnchor")?, "lookupAnchor")?,
	})
}

fn upward_message(
	value: &Value,
	caller: ParaId,
	codex: &mut Codex,
) -> Result<UpwardMessage, String> {
	let (tag, value) = variant(value)?;
	match tag {
		"Solicit" | "Forget" => {
			let len = u32::try_from(integer(field(value, "len")?)?)
				.map_err(|_| "preimage length out of range")?;
			let hash = codex.hash(integer(field(field(value, "hash")?, "hashBytes")?)?, len)?;
			let target = if let Some(target) = value.get("target") {
				let (kind, target) = variant(target)?;
				match kind {
					"Parachain" => Target::Parachain(para_id(target, codex)?),
					// Foreign service outcomes cannot be replayed until the host supports them.
					other => return Err(format!("unsupported preimage target {other}")),
				}
			} else if tag == "Solicit" {
				// Historical fixtures before the explicit Target vocabulary.
				Target::Parachain(caller)
			} else {
				Target::Parachain(para_id(field(value, "paraId")?, codex)?)
			};
			if tag == "Solicit" {
				Ok(UpwardMessage::Solicit { target, hash, len: Compact(len) })
			} else {
				Ok(UpwardMessage::Forget { target, hash, len: Compact(len) })
			}
		},
		"UpgradeService" => super::service_upgrade::message(value, codex),
		"TransferOut" => super::outgoing::message(value),
		"SetValidatorKeys" => Ok(UpwardMessage::SetValidatorKeys {
			keys: super::validator_keys::keys(field(value, "keys")?)?,
			is_last: boolean(field(value, "isLast")?)?,
		}),
		"AssignCore" => super::assignments::message(value),
		"SetKV" => {
			let key = bytes(field(value, "key")?)?;
			codex.register_kv_key(&key)?;
			Ok(UpwardMessage::SetKV { key, value: bytes(field(value, "value")?)? })
		},
		"RemoveKV" => Ok(UpwardMessage::RemoveKV {
			para_id: para_id(field(value, "paraId")?, codex)?,
			key: bytes(field(value, "key")?)?,
		}),
		"RequestCodeUpgrade" => {
			let len = integer(field(value, "len")?)?;
			let reference =
				codex.validation_code(integer(field(field(value, "hash")?, "vchBytes")?)?, len)?;
			let phase = match variant(field(value, "phase")?)? {
				("Announcement", _) => CodeUpgradePhase::Announcement,
				("Apply", _) => CodeUpgradePhase::Apply,
				(tag, _) => return Err(format!("unsupported code upgrade phase {tag}")),
			};
			Ok(UpwardMessage::RequestCodeUpgrade {
				hash: reference.hash,
				len: Compact(reference.len),
				phase,
			})
		},
		"ParachainSetStateBalance" => Ok(UpwardMessage::ParachainSetStateBalance {
			para_id: para_id(field(value, "paraId")?, codex)?,
			new_total: Compact(bounded_integer::<u64>(field(value, "newTotal")?, "newTotal")?),
		}),
		"ParachainSetHead" => Ok(UpwardMessage::ParachainSetHead {
			para_id: para_id(field(value, "paraId")?, codex)?,
			new_head: Codex::head(integer(field(value, "newHead")?)?)?,
		}),
		"ParachainCleanUp" => Ok(UpwardMessage::ParachainCleanUp(para_id(value, codex)?)),
		"ParachainSetValidationCode" => Ok(UpwardMessage::ParachainSetValidationCode {
			para_id: para_id(field(value, "paraId")?, codex)?,
			new_validation_code: seed::validation_code(field(value, "newValidationCode")?, codex)?,
		}),
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
		let len = bounded_integer::<u32>(&key[1], "preimage length")?;
		let blob = Codex::blob(abstract_hash, len)?;
		let expected_hash = codex.hash(abstract_hash, len)?;
		if hash_raw(&blob) != expected_hash {
			return Err("codex preimage hash mismatch".into());
		}
		storage
			.provide(bounded_integer::<u32>(field(current, "now")?, "now")?, MOCK_SERVICE_ID, &blob)
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
/// Convert model integers without wrapping negative or oversized values.
pub(crate) fn bounded_integer<T: TryFrom<i128>>(value: &Value, name: &str) -> Result<T, String> {
	let value = integer(value)?;
	T::try_from(value)
		.map_err(|_| format!("{name} out of {} range: {value}", std::any::type_name::<T>()))
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
pub(crate) fn para_id(value: &Value, codex: &mut Codex) -> Result<ParaId, String> {
	let (tag, value) = variant(value)?;
	if tag != "MkParaId" {
		return Err(format!("expected MkParaId, got {tag}"));
	}
	codex.register_para(integer(value)?)
}

/// Decode literal model bytes without truncating invalid integers.
pub(crate) fn bytes(value: &Value) -> Result<Vec<u8>, String> {
	value
		.as_array()
		.ok_or("expected byte list")?
		.iter()
		.map(|value| bounded_integer::<u8>(value, "byte"))
		.collect()
}

// Preserve host assigner ownership between frames, including after a handoff.
fn accumulate_block(
	storage: Storage,
	items: Vec<AccumulateItem>,
	slot: u32,
	privileges: jam_std_common::Privileges,
) -> Result<(executor::pj::AccumulateOutcome, Storage, jam_node::vm::StateMutations), String> {
	let engine = jam_node::vm::Engine::new(Some(jam_node::PvmBackend::Interpreter))
		.expect("interpreter engine should initialize");
	let mut context = parachain_service_bin::mock::accumulate_context_with_privileges(
		storage, items, slot, privileges,
	);
	let code_hash = context
		.storage
		.service(MOCK_SERVICE_ID)
		.expect("replay service exists")
		.code_hash;
	let outcome = executor::pj::accumulate(&engine, code_hash, &mut context)
		.map_err(|error| format!("accumulate installed service {code_hash:?}: {error}"))?;
	Ok((outcome, context.storage, context.mutations))
}

#[cfg(test)]
mod service_code_tests {
	use super::*;

	#[test]
	fn missing_installed_code_errors() {
		let storage = fresh_storage(|storage| {
			let mut service = storage.service(MOCK_SERVICE_ID).unwrap();
			service.code_hash = jam_types::CodeHash([99; 32]);
			storage.set_service(MOCK_SERVICE_ID, &service);
		});
		let privileges = super::super::assignments::initial_privileges(storage.clone());
		let error = accumulate_block(storage, Vec::new(), 1, privileges)
			.err()
			.expect("missing installed code must fail");
		assert!(error.contains("accumulate installed service"));
	}
}
