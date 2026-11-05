//! Differential Refine replay through the real service and a scripted child PVM.
use super::{
	codex::Codex,
	replay::{bounded_integer, field, integer, upward_message, variant, work_digest},
};
use crate::common::{work_item_skipped, work_item_with_auth_trace};
use codec::{DecodeAll, Encode};
use jam_node::vm::{RefineCallContext, Storage};
use jam_std_common::hash_raw;
use jam_types::AccumulateItem;
use parachain_service::{
	refine::ParachainCandidate,
	work_digest::{validation_code_hash, ParachainWorkDigest},
};
use parachain_service_bin::{
	authorizer_blob, blob,
	mock::{
		good_config_for, good_token, good_trace, refine_args, refine_work_item, MOCK_SERVICE_ID,
	},
};
use refine_replay_pvf::Call;
use serde_json::Value;

pub fn trace(document: &Value) -> Result<(), String> {
	let states = field(document, "states")?.as_array().ok_or("states must be a list")?;
	let first = states.first().ok_or("empty refine trace")?;
	if variant(field(first, "replayRefine")?)?.0 != "None" ||
		field(first, "replayRefineExpected")?.as_array().is_none_or(|v| !v.is_empty())
	{
		return Err("refine trace must start with an empty initialization".into());
	}
	for (index, state) in states.iter().enumerate().skip(1) {
		standalone(state).map_err(|e| format!("frame {index}: refine: {e}"))?;
	}
	Ok(())
}

fn standalone(state: &Value) -> Result<(), String> {
	let (tag, input) = variant(field(state, "replayRefine")?)?;
	if tag != "Some" {
		return Err("expected refine input".into());
	}
	let expected =
		field(state, "replayRefineExpected")?.as_array().ok_or("expected result list")?;
	if expected.len() != 1 {
		return Err("expected exactly one refine result".into());
	}
	compare(input, &expected[0], &mut Codex::default(), &blob()).map(|_| ())
}

/// Every integrated input must correspond to exactly one ordered work result.
/// The actual decoded digest, never the expected digest, becomes the operand.
pub fn work_items(
	storage: &Storage,
	current: &Value,
	results: &[Value],
	codex: &mut Codex,
) -> Result<Vec<AccumulateItem>, String> {
	if current.get("replayAuthorizationInputs").is_some() {
		return super::authorization::work_items(storage, current, results, codex);
	}
	let inputs = current.get("replayRefineInputs");
	let Some(inputs) = inputs else {
		return results.iter().map(|r| super::replay::work_item(r, codex)).collect();
	};
	let (tag, inputs) = variant(inputs)?;
	if tag == "None" {
		return results.iter().map(|r| super::replay::work_item(r, codex)).collect();
	}
	if tag != "Some" {
		return Err("invalid refine inputs option".into());
	}
	let inputs = inputs.as_array().ok_or("refine inputs must be a list")?;
	if inputs.len() != results.len() {
		return Err("refine input/work-result count differs".into());
	}
	let installed = storage.service(MOCK_SERVICE_ID).ok_or("missing refine service")?.code_hash;
	let service = storage
		.lookup(MOCK_SERVICE_ID, installed.0)
		.ok_or("missing installed refine code")?;
	inputs
		.iter()
		.zip(results)
		.enumerate()
		.map(|(index, (input, result))| {
			let actual = compare(input, field(result, "result")?, codex, &service)
				.map_err(|e| format!("work item {index}: refine: {e}"))?;
			let auth = Codex::auth_trace(integer(field(result, "authTrace")?)?)?;
			Ok(match actual {
				Some(digest) => work_item_with_auth_trace(&digest, auth),
				None => work_item_skipped(auth),
			})
		})
		.collect()
}

pub fn validate(document: &Value, states: &[Value]) -> Result<(), String> {
	super::authorization::validate(document, states)?;
	let enabled = states.first().is_some_and(|s| s.get("replayRefineInputs").is_some());
	for state in states {
		if state.get("replayRefineInputs").is_some() != enabled {
			return Err("refine inputs must be present in every frame".into());
		}
	}

	let required = document
		.get("replayRefineRequired")
		.map(|v| v.as_bool().ok_or("replayRefineRequired must be boolean"))
		.transpose()?
		.unwrap_or(false);
	if required {
		if !enabled {
			return Err("combined replay requires refine inputs".into());
		}
		for pair in states.windows(2) {
			if super::classify::classify(&pair[0], &pair[1])? == super::classify::FrameKind::Block {
				let results =
					field(&pair[1], "lastStepWorkResults")?.as_array().ok_or("expected results")?;
				if !results.is_empty() &&
					variant(field(&pair[1], "replayRefineInputs")?)?.0 != "Some"
				{
					return Err(
						"combined replay requires refine inputs for every work result".into()
					);
				}
			}
		}
	}
	Ok(())
}

fn compare(
	input: &Value,
	expected: &Value,
	codex: &mut Codex,
	service: &[u8],
) -> Result<Option<ParachainWorkDigest>, String> {
	let (tag, value) = variant(expected)?;
	// JAM can reject work before the service's refine entry point runs.
	let skip = input
		.get("skip")
		.map(|v| v.as_bool().ok_or("skip must be boolean"))
		.transpose()?
		.unwrap_or(false);
	let actual = if skip { None } else { execute(input, codex, service)? };
	match (tag, &actual) {
		("WorkErr", None) => Ok(actual),
		("WorkOk", Some(digest)) => {
			let expected = work_digest(value, codex)?;
			if *digest != expected {
				return Err(format!("digest differs: expected {expected:?}, got {digest:?}"));
			}
			Ok(actual)
		},
		_ => Err(format!("expected {tag}, got {actual:?}")),
	}
}

fn execute(
	input: &Value,
	codex: &mut Codex,
	service: &[u8],
) -> Result<Option<ParachainWorkDigest>, String> {
	execute_authorized(input, codex, service, None).map(|(digest, _)| digest)
}

pub(super) fn execute_authorized(
	input: &Value,
	codex: &mut Codex,
	service: &[u8],
	fault: Option<&str>,
) -> Result<(Option<ParachainWorkDigest>, Option<jam_types::AuthTrace>), String> {
	let pkg = field(input, "pkg")?;
	let paras = field(pkg, "authorizedParas")?.as_array().ok_or("expected para list")?;
	let paras = paras
		.iter()
		.map(|p| super::replay::para_id(p, codex))
		.collect::<Result<Vec<_>, _>>()?;
	let items = field(pkg, "items")?.as_array().ok_or("expected item list")?;
	if !(1..=2).contains(&items.len()) {
		return Err("refine replay supports one or two items".into());
	}
	// The interpreter PVF stands in for abstract validation code. Only its
	// identity is rebound on the returned digest, after checking the real hash.
	// Model lengths remain preimage-registry keys; they are not executable blobs.
	let reference = field(&items[0], "validationCode")?;
	for item in items {
		if field(item, "validationCode")? != reference {
			return Err("multi-item fixture requires identical code references".into());
		}
	}
	bounded_integer::<u32>(field(reference, "len")?, "code length")?;

	let caller = paras.first().copied().unwrap_or(Codex::para_id(0)?);
	let calls = field(input, "calls")?.as_array().ok_or("expected call list")?;
	let mut script = Vec::new();
	for call in calls {
		let (tag, value) = variant(call)?;
		script.push(match tag {
			"SetParentHeadHash" => {
				Call::Parent(hash_raw(&Codex::head(integer(field(value, "headBytes")?)?)?))
			},
			"SetHead" => Call::Head(Codex::head(integer(value)?)?.to_vec()),
			"SetHeadOversized" => Call::Head(vec![0; 4097]),
			"Send" => Call::Send(upward_message(value, caller, codex)?.encode()),
			"ReportError" => {
				let len = bounded_integer::<usize>(value, "error payload")?;
				if len > 1024 {
					return Err("error payload exceeds model domain".into());
				}
				Call::Error(vec![0xaa; len])
			},
			"Panic" => Call::Panic,
			_ => return Err(format!("unsupported call {tag}")),
		});
	}
	let gas_mode = input
		.get("gasMode")
		.map(|v| v.as_str().ok_or("gasMode must be string"))
		.transpose()?
		.unwrap_or("normal");
	match gas_mode {
		"normal" | "zero" | "funded" | "cutoff" => {},
		"child" => script.push(Call::ExhaustGas),
		_ => return Err(format!("unsupported gasMode {gas_mode}")),
	}
	let abstract_code = codex.code_hash(integer(field(field(reference, "hash")?, "vchBytes")?)?)?;
	let byte_fault = input
		.get("byteFault")
		.map(|v| v.as_str().ok_or("byteFault must be string"))
		.transpose()?
		.unwrap_or("valid");
	let mut pvf = cargo_jam_build::blob("refine-replay-pvf").to_vec();
	match byte_fault {
		"valid" | "candidate_empty" | "candidate_truncated" | "candidate_trailing" => {},
		"pvf_empty" => pvf.clear(),
		"pvf_magic" => pvf[0] ^= 0xff,
		"pvf_truncated" => pvf.truncate(4),
		"pvf_memory" | "pvf_entry" => {
			use polkavm_common::{
				program::{Instruction, InstructionSetKind},
				writer::ProgramBlobBuilder,
			};
			let mut builder = ProgramBlobBuilder::new(InstructionSetKind::JamV1);
			builder.set_code(&[Instruction::trap], &[]);
			if byte_fault == "pvf_memory" {
				builder.set_rw_data_size(u32::MAX);
				builder.add_export_by_instruction(
					0,
					parachain_service::pvf::PVF_ENTRY_POINT.as_bytes(),
				);
			}
			pvf = builder.into_vec().map_err(|e| format!("fault container: {e}"))?;
			// Both fixtures must parse as containers; only the intended semantic
			// validation fails. Keep this separate from the Quint outcome oracle.
			polkavm::ProgramBlob::parse(pvf.clone().into())
				.map_err(|e| format!("fault container does not parse: {e}"))?;
			let parsed = parachain_service::pvf::pvm::parse_pvf(&pvf);
			let intended = matches!(
				(&parsed, byte_fault),
				(
					Err(parachain_service::pvf::pvm::PvfParseError::InvalidMemoryMap(_)),
					"pvf_memory"
				) | (
					Err(parachain_service::pvf::pvm::PvfParseError::MissingEntryPoint),
					"pvf_entry"
				)
			);
			if !intended {
				return Err("fault container missed intended parse failure".into());
			}
		},
		_ => return Err(format!("unsupported byteFault {byte_fault}")),
	}
	let code = validation_code_hash(&pvf);
	let mut payload = ParachainCandidate { validation_code: code }.encode();
	match byte_fault {
		"candidate_empty" => payload.clear(),
		"candidate_truncated" => {
			payload.pop();
		},
		"candidate_trailing" => payload.push(0),
		_ => {},
	}
	let work = (0..items.len())
		.map(|_| refine_work_item(service, payload.clone(), vec![script.encode()]))
		.collect();
	let available = field(input, "available")?.as_bool().ok_or("available must be boolean")?;
	let missing = field(input, "missingPrefix")?
		.as_bool()
		.ok_or("missingPrefix must be boolean")?;
	let config =
		if missing { jam_types::AuthConfig(Default::default()) } else { good_config_for(paras) };
	let preimages: Vec<&[u8]> = if available { vec![&pvf] } else { vec![] };
	let (engine, service_hash, mut context) = refine_args(
		service,
		&authorizer_blob(),
		config,
		good_token(),
		good_trace(),
		work,
		&preimages,
		0,
	);
	match gas_mode {
		"zero" => context.gas = 0,
		"child" | "funded" => context.gas = 1_000_000,
		_ => {},
	}
	// The immutable package must be rebuilt after changing its lookup anchor.
	let encoded = jam_codec::Encode::encode(&context.work_package);
	let mut package: jam_types::WorkPackage = jam_codec::Decode::decode(&mut &encoded[..])
		.map_err(|e| format!("package decode: {e:?}"))?;
	package.context.lookup_anchor_slot = bounded_integer(field(input, "anchor")?, "anchor")?;
	let auth = if let Some(fault) = fault {
		let Some(trace) = super::authorization::authorize(&mut package, fault)? else {
			return Ok((None, None));
		};
		context.auth_output = trace.clone();
		if fault == "trace_empty" {
			context.auth_output.0.clear();
		}
		if fault == "trace_trailing" {
			context.auth_output.0.push(0);
		}
		if fault == "trace_truncated" {
			context.auth_output.0.pop();
		}
		Some(trace)
	} else {
		None
	};
	context.work_package = package.into();
	if gas_mode == "cutoff" {
		if !available || missing || byte_fault != "valid" || fault.is_some() {
			return Err("cutoff requires available valid code and unmodified authorization".into());
		}
		let sample = bounded_integer::<u64>(field(input, "gasSample")?, "gas sample")?;
		if sample > 1_000_000 {
			return Err("gas sample exceeds 1000000".into());
		}
		// Calibrate only cost, never the expected failure or Accumulate state.
		// Quint has already selected WorkErr. Require a terminating successful
		// control so a pre-existing trap cannot masquerade as gas coverage.
		let (control, _, used) = engine.refine(service_hash, RefineCallContext::from(&mut context));
		let output = control.map_err(|e| format!("cutoff control failed: {e:?}"))?;
		if !matches!(
			ParachainWorkDigest::decode_all(&mut &output[..]),
			Ok(ParachainWorkDigest::Ok { .. })
		) || used == 0
		{
			return Err("cutoff control must produce a successful digest with nonzero gas".into());
		}
		context.output_len = 0;
		context.gas = (((used - 1) as u128 * sample as u128) / 1_000_000) as u64;
	}
	let (actual, _, _) = engine.refine(service_hash, RefineCallContext::from(&mut context));
	match actual {
		// The hazmat API does not expose the error enum. Compare its exact
		// variant, as the accumulate recovery adapter does. Only explicit gas
		// scenarios admit exhaustion; ordinary traces still reject it.
		Err(error) if format!("{error:?}") == "Trap" => Ok((None, auth)),
		Err(error)
			if gas_mode != "normal" &&
				gas_mode != "funded" &&
				format!("{error:?}") == "NotEnoughGas" =>
		{
			Ok((None, auth))
		},
		Err(error) => Err(format!("unexpected refine VM failure: {error:?}")),
		Ok(output) => {
			let mut actual = ParachainWorkDigest::decode_all(&mut &output[..])
				.map_err(|e| format!("invalid Rust digest: {e}"))?;
			let validation_code = match &mut actual {
				ParachainWorkDigest::Ok { validation_code, .. } |
				ParachainWorkDigest::Err { validation_code, .. } => validation_code,
			};
			if *validation_code != code {
				return Err("Rust digest names the wrong executable code".into());
			}
			*validation_code = abstract_code;
			Ok((Some(actual), auth))
		},
	}
}
