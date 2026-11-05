//! Real IsAuthorized → Refine → Accumulate, with an independent Quint admission oracle.
use super::{
	codex::Codex,
	replay::{field, integer, variant, work_digest},
};
use crate::common::{work_item_skipped, work_item_with_auth_trace};
use codec::{DecodeAll, Encode};
use ed25519_dalek::{Signer, SigningKey};
use jam_node::vm::Storage;
use jam_types::{AccumulateItem, AuthTrace, WorkPackage};
use parachain_authorizer::aura::{signable_work_package_hash, AuthConfig, AuthToken};
use parachain_service_bin::{
	authorizer_blob,
	mock::{is_authorized_args, MOCK_SERVICE_ID},
};
use serde_json::Value;

const SEED: [u8; 32] = [0x42; 32];

pub(super) fn expected_trace() -> AuthTrace {
	AuthTrace(SigningKey::from_bytes(&SEED).verifying_key().to_bytes().to_vec())
}

/// Sign the final package, including its lookup anchor, before applying faults.
/// Only guest traps count as rejection; loading and gas errors fail the replay.
pub(super) fn authorize(
	package: &mut WorkPackage,
	fault: &str,
) -> Result<Option<AuthTrace>, String> {
	let mut config = AuthConfig::decode_all(&mut &package.authorizer.config.0[..])
		.map_err(|e| format!("authorization config: {e}"))?;
	match fault {
		"wrong_service" => config.parachain_service = MOCK_SERVICE_ID + 1,
		"para_count" => config.para_ids.clear(),
		_ => {},
	}
	package.authorizer.config.0 = config.encode();
	// Corrupt the encoded u32 fields: NonZero prevents constructing invalid configs.
	let len = package.authorizer.config.0.len();
	match fault {
		"zero_collators" => package.authorizer.config.0[len - 8..len - 4].fill(0),
		"zero_duration" => package.authorizer.config.0[len - 4..].fill(0),
		_ => {},
	}
	let key = SigningKey::from_bytes(&SEED);
	let mut token = AuthToken {
		proof: vec![],
		key: key.verifying_key().to_bytes(),
		signature: key.sign(signable_work_package_hash(package).as_bytes()).to_bytes(),
	};
	match fault {
		"signature" => token.signature[0] ^= 1,
		"proof" => token.key[0] ^= 1,
		_ => {},
	}
	package.authorization.0 = token.encode();
	match fault {
		"valid" | "signature" | "proof" | "wrong_service" | "para_count" | "zero_collators" |
		"zero_duration" | "trace_empty" | "trace_trailing" | "trace_truncated" => {},
		"config_empty" => package.authorizer.config.0.clear(),
		"config_truncated" => {
			package.authorizer.config.0.pop();
		},
		"config_trailing" => package.authorizer.config.0.push(0),
		"token_empty" => package.authorization.0.clear(),
		"token_truncated" => {
			package.authorization.0.pop();
		},
		"token_trailing" => package.authorization.0.push(0),
		"signed_context" => package.context.anchor_slot += 1,
		other => return Err(format!("unsupported authorization fault {other}")),
	}
	// This helper provisions the actual authorizer and its host parameters.
	let (engine, _, storage) = is_authorized_args(
		&authorizer_blob(),
		package.authorizer.config.clone(),
		package.authorization.clone(),
		package.items.to_vec(),
	);
	let (outcome, _) = engine.is_authorized(&package.clone().into(), 0, &storage, None);
	match outcome {
		Ok(trace) if trace == expected_trace() => Ok(Some(trace)),
		Ok(trace) => Err(format!("authorization trace differs: {trace:?}")),
		Err(error) if format!("{error:?}") == "Trap" => Ok(None),
		Err(error) => Err(format!("unexpected authorization VM failure: {error:?}")),
	}
}

pub(super) fn work_items(
	storage: &Storage,
	current: &Value,
	results: &[Value],
	codex: &mut Codex,
) -> Result<Vec<AccumulateItem>, String> {
	let entries = field(current, "replayAuthorizationInputs")?
		.as_array()
		.ok_or("authorization inputs must be a list")?;
	let (tag, refined) = variant(field(current, "replayRefineInputs")?)?;
	if tag != "Some" {
		return Err("authorization requires refine inputs".into());
	}
	let refined = refined.as_array().ok_or("refine inputs must be a list")?;
	let installed = storage.service(MOCK_SERVICE_ID).ok_or("missing refine service")?.code_hash;
	let service = storage.lookup(MOCK_SERVICE_ID, installed.0).ok_or("missing refine code")?;
	let mut output = Vec::new();
	for entry in entries {
		let input = field(entry, "input")?;
		if field(input, "skip")?.as_bool() != Some(false) {
			return Err("authorization replay cannot skip execution".into());
		}
		let fault = field(entry, "fault")?.as_str().ok_or("fault must be a string")?;
		let accepted = field(entry, "accepted")?.as_bool().ok_or("accepted must be boolean")?;
		let (digest, auth) =
			super::refine::execute_authorized(input, codex, &service, Some(fault))?;
		if auth.is_some() != accepted {
			return Err(format!("authorization admission differs for {fault}"));
		}
		let Some(auth) = auth else {
			continue;
		};
		let index = output.len();
		if refined.get(index) != Some(input) {
			return Err("authorized refine input differs".into());
		}
		let result = results.get(index).ok_or("authorization work-result count differs")?;
		if auth != Codex::auth_trace(integer(field(result, "authTrace")?)?)? {
			return Err("authorization report trace differs".into());
		}
		let (tag, expected) = variant(field(result, "result")?)?;
		output.push(match (tag, digest) {
			("WorkOk", Some(actual)) if actual == work_digest(expected, codex)? => {
				work_item_with_auth_trace(&actual, auth)
			},
			("WorkErr", None) => work_item_skipped(auth),
			_ => return Err("authorized refine digest differs".into()),
		});
	}
	if output.len() != results.len() || output.len() != refined.len() {
		return Err("authorization work-result count differs".into());
	}
	Ok(output)
}

pub(super) fn validate(document: &Value, states: &[Value]) -> Result<(), String> {
	let required = document
		.get("replayAuthorizationRequired")
		.and_then(Value::as_bool)
		.unwrap_or(false);
	let enabled = states.first().is_some_and(|s| s.get("replayAuthorizationInputs").is_some());
	if required && !enabled {
		return Err("authorization replay requires authorization inputs".into());
	}
	for state in states {
		if state.get("replayAuthorizationInputs").is_some() != enabled {
			return Err("authorization inputs must be present in every frame".into());
		}
	}
	if enabled {
		if !field(&states[0], "replayAuthorizationInputs")?
			.as_array()
			.is_some_and(Vec::is_empty)
		{
			return Err("authorization initialization must be empty".into());
		}
		for pair in states.windows(2) {
			if super::classify::classify(&pair[0], &pair[1])? != super::classify::FrameKind::Block {
				return Err("authorization inputs require an invocation frame".into());
			}
		}
	}
	Ok(())
}
