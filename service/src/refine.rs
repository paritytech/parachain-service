//! `refine` entry point of the parachain service.

use crate::work_digest::{ParachainWorkDigest, RefineLog, ValidationCodeHash, ValidationCodeRef};
use alloc::{vec, vec::Vec};
use codec::{Decode, DecodeAll, Encode};
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
	// TODO: check if its actually invalid per GP
	assert!(item_index < work_items.len(), "Out of bounds item_index is invalid per GP");

	// TODO: The quint spec uses default 0 here, but why?
	let para_id = *para_ids.get(0).unwrap_or(&ParaId::from(0));

	let Ok([work_item]): Result<&[_; 1], _> = work_items.as_slice().try_into() else {
		// TODO: maybe reorder this with the check below, quint does it this way for some reason
		return ParachainWorkDigest::Err { para_id, error: RefineLog::InvalidItemCount };
	};

	if work_items.len() != para_ids.len() {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::AuthConfigMismatch };
	};

	let Ok(candidate) = ParachainCandidate::decode_all(&mut &raw_payload.0[..]) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::MalformedPayload };
	};

	let code_hash = candidate.validation_code_hash;
	let Some(code) = refine::lookup(&code_hash.0) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::InvalidCodeHash };
	};
	let Ok(code_len) = TryInto::<u32>::try_into(code.len()) else {
		// NOTE: Should be impossible, but still nicer that panicking.
		// FIXME: Own error variant
		return ParachainWorkDigest::Err { para_id, error: RefineLog::TooBigCode };
	};

	// FIXME: call into PVF
	let head_data = vec![];

	ParachainWorkDigest::Ok {
		para_id,
		validation_code: ValidationCodeRef { hash: code_hash, len: code_len },
		parent_head_hash: [0; 32], // FIXME
		head_data,
		upward_messages: vec![],
		lookup_anchor: 123, // FIXME
	}
}
