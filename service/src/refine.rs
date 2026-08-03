//! `refine` entry point of the parachain service.

use crate::work_digest::{ParachainWorkDigest, RefineLog, ValidationCodeHash, ValidationCodeRef};
use alloc::{vec, vec::Vec};
use codec::{Decode, DecodeAll, Encode};
use jam_pvm_common::refine::{self, auth_trace};
use jam_types::{CoreIndex, ServiceId, WorkPackageHash, WorkPayload};
use parachain_support::types::ParaId;

/// Work package payload for a parachain candidate.
#[derive(Encode, Decode)]
struct CandidatePayload {
	validation_code_hash: ValidationCodeHash,
}

pub fn refine(
	_core_index: CoreIndex,
	item_index: usize,
	_service_id: ServiceId,
	raw_payload: WorkPayload,
	_package_hash: WorkPackageHash,
) -> ParachainWorkDigest {
	let _auth_trace = auth_trace();
	let auth_config = refine::work_package().authorizer.config;

	let Ok(para_ids) = Vec::<ParaId>::decode(&mut &auth_config[..]) else {
		return ParachainWorkDigest::AuthError {
			// FIXME: panic here or return error?
			error: RefineLog::MalformedAuthorizerConfig,
		};
	};
	// NOTE: We simplify the spec here and require exactly one ParaId and WorkItem.
	let Ok([para_id]): Result<&[_; 1], _> = para_ids.as_slice().try_into().clone() else {
		return ParachainWorkDigest::AuthError { error: RefineLog::AuthConfigMismatch };
	};

	let Ok(candidate) = CandidatePayload::decode_all(&mut &raw_payload.0[..]) else {
		// FIXME: add dedicated error for this case
		return ParachainWorkDigest::Err { para_id: *para_id, error: RefineLog::InvalidCodeHash };
	};

	let work_items = refine::work_items_summary();
	let Ok([work_item]): Result<&[_; 1], _> = work_items.as_slice().try_into() else {
		return ParachainWorkDigest::Err { para_id: *para_id, error: RefineLog::InvalidItemCount };
	};

	// TODO: check if its actually invalid per GP
	assert_eq!(item_index, 0, "Out of bounds item_index is invalid per GP");

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
		return ParachainWorkDigest::Err { para_id: *para_id, error: RefineLog::InvalidCodeHash };
	};

	// FIXME: call into PVM
	let head_data = vec![];

	ParachainWorkDigest::Ok {
		para_id: *para_id,
		validation_code: ValidationCodeRef {
			hash: code_hash,
			len: code.len().try_into().expect("Code is less than 4 GiB"),
		},
		parent_head_hash: [0; 32], // FIXME
		head_data,
		upward_messages: vec![],
		lookup_anchor: 123, // FIXME
	}
}
