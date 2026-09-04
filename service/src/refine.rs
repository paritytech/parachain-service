//! `refine` entry point of the parachain service.

use crate::{
	pvf,
	work_digest::{ParachainWorkDigest, RefineLog, ValidationCodeRef},
};
use alloc::vec::Vec;
use codec::{Decode, DecodeAll};
use jam_pvm_common::refine::{self, auth_trace, lookup as historical_lookup};
use jam_types::{CoreIndex, ServiceId, WorkPackageHash, WorkPayload};
use parachain_authorizer::aura;
use parachain_service_interface::types::ParaId;

pub use parachain_service_interface::candidate::ParachainCandidate;

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

	let work_items = refine::work_items_summary();
	assert!(item_index < work_items.len(), "Out of bounds item_index is invalid per GP");

	// Package-level failures are all settled before a `para_id` becomes
	// authoritative, so none of them can name a para to log against and all
	// panic (§4.2). A config that will not decode at all never gets here:
	// `is_authorized` fails on it first, so it never becomes a work report.
	assert_eq!(work_items.len(), para_ids.len(), "AuthConfig must name one para per work item");
	let Ok([_work_item]): Result<&[_; 1], _> = work_items.as_slice().try_into() else {
		panic!("Only single-item work packages are supported")
	};
	let para_id = para_ids[item_index];

	// The authorizer deploys as its own blob, so `is_authorized` vouches for
	// the trace's contract, not for ours: a shape this service cannot decode
	// is logged against the para instead of trapping the whole refine.
	if aura::AuthTrace::decode_all(&mut &raw_auth_trace[..]).is_err() {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::MalformedAuthTrace };
	}

	let Ok(candidate) = ParachainCandidate::decode_all(&mut &raw_payload.0[..]) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::MalformedPayload };
	};

	let code_hash = candidate.validation_code_hash;
	let Some(code) = historical_lookup(&code_hash.0) else {
		return ParachainWorkDigest::Err { para_id, error: RefineLog::InvalidCodeHash };
	};
	let code_len: u32 = code.len().try_into().expect("PVF code must be at most 4 GiB");

	// An unparseable PVF is an abnormal exit: it fails the whole refine
	// invocation, not the digest (§4.2).
	let Ok(parsed) = pvf::pvm::parse_pvf(&code) else {
		panic!("PVF code could not be parsed as a PVM program; §4.2 whole-refine failure")
	};
	let (parent_head_hash, head_data, upward_messages) = match pvf::pvm::run(&parsed, para_id) {
		Ok(ok) => ok,
		Err(error) => return ParachainWorkDigest::Err { para_id, error },
	};

	ParachainWorkDigest::Ok {
		para_id,
		validation_code: ValidationCodeRef { hash: code_hash, len: code_len },
		parent_head_hash,
		head_data,
		upward_messages,
		lookup_anchor: refine::refine_context().lookup_anchor_slot,
	}
}
