//! `refine` entry point of the parachain service.

use crate::work_digest::{ParachainWorkDigest, RefineLog, ValidationCodeHash, ValidationCodeRef};
use alloc::{vec, vec::Vec};
use codec::{Decode, DecodeAll, Encode};
use jam_pvm_common::refine::lookup as historical_lookup; // PolkaJAM somehow renamed the
														 // export
use jam_pvm_common::refine::{self, auth_trace};
use jam_types::{CoreIndex, ServiceId, WorkPackageHash, WorkPayload};
use parachain_authorizer::aura;
use parachain_support::types::ParaId;

/// Work package payload for a parachain candidate.
#[derive(Encode, Decode)]
pub struct ParachainCandidate {
	/// The hash of the currently active validation code. Used by Refine to
	/// look up the PVF bytecode from the preimage store.
	pub validation_code_hash: ValidationCodeHash,

	/// The Proof-of-Validity (PoV) — the actual block data + witness.
	pub pov: Vec<u8>,
}

pub fn refine(
	_core_index: CoreIndex,
	item_index: usize,
	_service_id: ServiceId,
	raw_payload: WorkPayload,
	_package_hash: WorkPackageHash,
) -> ParachainWorkDigest {
	let raw_auth_trace = auth_trace();
	let raw_auth_config = refine::work_package().authorizer.config;

	let Ok(para_ids) = Vec::<ParaId>::decode(&mut &raw_auth_config[..]) else {
		panic!("The AuthConfig already passed IsAuthorized, it must be valid")
	};
	let Ok(_auth_trace) = aura::AuthTrace::decode_all(&mut &raw_auth_trace[..]) else {
		panic!("The AuthTrace was produced by IsAuthorized, it must be valid")
	};

	let work_items = refine::work_items_summary();
	assert!(item_index < work_items.len(), "Out of bounds item_index is invalid per GP");

	// TODO: The quint spec uses default 0 here, but why?
	let para_id = *para_ids.get(item_index).unwrap_or(&ParaId::from(0));

	if work_items.len() != para_ids.len() {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::AuthConfigMismatch };
	};

	let Ok([work_item]): Result<&[_; 1], _> = work_items.as_slice().try_into() else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::InvalidItemCount };
	};

	let Ok(candidate) = ParachainCandidate::decode_all(&mut &raw_payload.0[..]) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::MalformedPayload };
	};

	let code_hash = candidate.validation_code_hash;
	let Some(code) = historical_lookup(&code_hash.0) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::InvalidCodeHash };
	};
	let code_len: u32 = code.len().try_into().expect("PVF code must be at most 4 GiB");

	// Preparing for inner PVM invocation:

	let Some(pvf) = parse_pvf(&code) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::InvalidCode };
	};
	let Ok(vm_handle) = refine::machine(&pvf.code[..], pvf.entry_pc) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::InvalidCode };
	};

	let _ = vm_handle; // TODO: invoke `vm_handle` and collect head_data / upward_messages.

	let head_data = vec![]; // FIXME

	ParachainWorkDigest::Ok {
		para_id,
		validation_code: ValidationCodeRef { hash: code_hash, len: code_len },
		parent_head_hash: [0; 32], // FIXME
		head_data,
		upward_messages: vec![],
		lookup_anchor: 123, // FIXME
	}
}

/// A parachain validation function parsed into a form ready to run as an inner PVM.
struct ParsedPvf {
	/// The `code + jump table` blob in JAM's standard program format, as the `machine`
	/// host call expects it.
	code: polkavm::ArcBytes,
	/// Program counter of the `validate_block` export: the inner PVM's entry point.
	entry_pc: u64,
}

// TODO: let the code hash already commit to this instead of the bare code
fn parse_pvf(code: &[u8]) -> Option<ParsedPvf> {
	use polkavm::{ArcBytes, ProgramBlob, ProgramParts};

	let parts = ProgramParts::from_bytes(ArcBytes::from(code)).ok()?;
	let code = parts.code_and_jump_table.clone();

	let program = ProgramBlob::from_parts(parts).ok()?;
	let entry_pc = program.exports().find(|export| export == "validate_block")?.program_counter().0;

	Some(ParsedPvf { code, entry_pc: entry_pc as u64 })
}
