//! `refine` entry point of the parachain service.

use crate::{
	pvf,
	work_digest::{ParachainWorkDigest, RefineLog},
};
use codec::DecodeAll;
use jam_pvm_common::refine::{self, auth_trace, lookup as historical_lookup};
use jam_types::{CoreIndex, ServiceId, WorkPackageHash, WorkPayload};
use parachain_service_core::{authorization::AuthTrace, types::validation_code_hash_bytes};

pub use parachain_service_core::candidate::ParachainCandidate;

pub fn refine(
	_core_index: CoreIndex,
	item_index: usize,
	_service_id: ServiceId,
	raw_payload: WorkPayload,
	_package_hash: WorkPackageHash,
) -> ParachainWorkDigest {
	let raw_auth_trace = auth_trace();
	let Ok(_auth_trace) = AuthTrace::decode_all(&mut &raw_auth_trace[..]) else {
		panic!("The AuthTrace was produced by IsAuthorized, it must be valid")
	};

	let work_items = refine::work_items_summary();
	assert!(item_index < work_items.len(), "Out of bounds item_index is invalid per GP");

	// The authorizer validates each candidate's parachain identity. Refine only
	// supports single-item packages (§3.2); a violation traps without a digest.
	let Ok([_work_item]): Result<&[_; 1], _> = work_items.as_slice().try_into() else {
		panic!("Only single-item work packages are supported")
	};

	let Ok(candidate) = ParachainCandidate::decode_all(&mut &raw_payload.0[..]) else {
		panic!("Work item payload must decode as a ParachainCandidate (§4.1 step 1)")
	};
	let para_id = candidate.para_id;

	let validation_code = candidate.validation_code;
	let Some(code) = historical_lookup(&validation_code_hash_bytes(&validation_code)) else {
		return ParachainWorkDigest::Err {
			para_id,
			validation_code,
			error: RefineLog::ValidationCodeLookupFailed,
		};
	};

	// An unparseable PVF is an abnormal exit: it fails the whole refine
	// invocation, not the digest (§4.2).
	let Ok(parsed) = pvf::pvm::parse_pvf(&code) else {
		panic!("PVF code could not be parsed as a PVM program; §4.2 whole-refine failure")
	};
	let (parent_head_hash, head_data, upward_messages) = match pvf::pvm::run(&parsed, para_id) {
		Ok(ok) => ok,
		Err(error) => return ParachainWorkDigest::Err { para_id, validation_code, error },
	};

	ParachainWorkDigest::Ok {
		para_id,
		validation_code,
		parent_head_hash,
		head_data,
		upward_messages,
		lookup_anchor: refine::refine_context().lookup_anchor_slot,
	}
}
