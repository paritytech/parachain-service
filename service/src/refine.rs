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
pub struct CandidatePayload {
	pub validation_code_hash: ValidationCodeHash,
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
	let Ok(auth_trace) = aura::AuthTrace::decode_all(&mut &raw_auth_trace[..]) else {
		panic!("The AuthTrace was produced by IsAuthorized, it must be valid")
	};

	let work_items = refine::work_items_summary();
	// TODO: check if its actually invalid per GP
	assert!(item_index < work_items.len(), "Out of bounds item_index is invalid per GP");
	let para_id = para_ids.get(item_index).expect("There must be a para_id for each work_item");

	if work_items.len() != para_ids.len() {
		return ParachainWorkDigest::Err {
			para_id: *para_id,
			error: RefineLog::AuthConfigMismatch,
		};
	};

	let Ok([work_item]): Result<&[_; 1], _> = work_items.as_slice().try_into() else {
		return ParachainWorkDigest::Err { para_id: *para_id, error: RefineLog::InvalidItemCount };
	};

	let Ok(candidate) = CandidatePayload::decode_all(&mut &raw_payload.0[..]) else {
		return ParachainWorkDigest::Err { para_id: *para_id, error: RefineLog::MalformedPayload };
	};

	if work_item.extrinsics_count != 2 {
		return ParachainWorkDigest::Err {
			para_id: *para_id,
			error: RefineLog::InvalidExtrinsicCount,
		};
	}

	// TODO: check if we should load them chunked to not OOM
	let _ext_para_state_proof = refine::extrinsic(0).expect("checked above");
	let _ext_jam_state_proof = refine::extrinsic(1).expect("checked above");

	let code_hash = candidate.validation_code_hash;
	let Some(code) = refine::lookup(&code_hash.0) else {
		return ParachainWorkDigest::Err {
			para_id: *para_id,
			error: RefineLog::UnrequestedCodeHash,
		};
	};
	let Ok(code_len) = TryInto::<u32>::try_into(code.len()) else {
		// NOTE: Should be impossible, but still nicer that panicking.
		// FIXME: Own error variant
		return ParachainWorkDigest::Err { para_id: *para_id, error: RefineLog::TooBigCode };
	};

	// FIXME: call into PVF
	let head_data = vec![];

	ParachainWorkDigest::Ok {
		para_id: *para_id,
		validation_code: ValidationCodeRef { hash: code_hash, len: code_len },
		parent_head_hash: [0; 32], // FIXME
		head_data,
		upward_messages: vec![],
		lookup_anchor: 123, // FIXME
	}
}
