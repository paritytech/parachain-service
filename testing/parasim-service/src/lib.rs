//! parasim — a real JAM service with fake logic (spec B.0 item 8).
//!
//! Accepts a parachain work package without verifying the PoV, extracts the new
//! para head from the payload (`ParachainBlockData`), and upserts it into this
//! service's own key–value store under the real `parachain-service` storage-key
//! layout — tag `0x00` + SCALE(`ParaId`) → a byte-exact `ParaInfo` whose only
//! meaningful field is `head_data`. The collator code that reads the para head
//! via `serviceValue` carries over unchanged to the real service.
//!
//! Deliberately NOT the real service: no PVF, no log, no kv storage semantics,
//! no transfers, no upgrade tracking, no §5.5 head commitment. Deliberately a
//! separate crate (not a mode of `parachain-service`, which is churning on the
//! POC branch).

#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use codec::{Decode, DecodeAll, Encode};
use jam_pvm_common::{declare_service, Service};
use jam_types::{
	CoreIndex, Hash, ServiceId, Slot, WorkOutput, WorkPackageHash, WorkPayload,
};
use parachain_service_interface::types::{HeadData, ParaId};

pub mod pov;

/// Directory of this crate's `Cargo.toml`, used by `parasim-service/bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// The `ParaId` parasim falls back to when the authorizer config does not pin
/// one. Phases 1–2 run under the null authorizer (empty config), which decodes
/// to no paras; without a fallback parasim would reject every package. Mock-only;
/// removed once a real authorizer config arrives in phase 3.
pub const FALLBACK_PARA_ID: ParaId = ParaId(0);

/// What parasim's `refine` hands to `accumulate` for one work item:
/// `(para_id, head_data)`. Accumulate cannot see the work-item payload, so the
/// head (and the para it belongs to) travel in the work output.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ParasimWorkOutput {
	pub para_id: ParaId,
	pub head_data: HeadData,
}

pub struct ParasimService;
declare_service!(ParasimService);

impl Service for ParasimService {
	fn refine(
		_core_index: CoreIndex,
		item_index: usize,
		_service_id: ServiceId,
		payload: WorkPayload,
		_package_hash: WorkPackageHash,
	) -> WorkOutput {
		match refine_inner(item_index, &payload) {
			Ok(output) => WorkOutput(output.encode()),
			Err(error) => {
				jam_pvm_common::error!("parasim: refine failed: {error:?}");
				WorkOutput(error.encode())
			},
		}
	}

	fn accumulate(_slot: Slot, _id: ServiceId, _item_count: usize) -> Option<Hash> {
		for item in jam_pvm_common::accumulate::accumulate_items() {
			if let jam_types::AccumulateItem::WorkItem(record) = item {
				let Ok(result) = record.result else { continue };
				accumulate_one(&result);
			}
		}
		None
	}
}

/// Decode, extract, and re-emit the head for one work item.
fn refine_inner(
	item_index: usize,
	payload: &WorkPayload,
) -> Result<ParasimWorkOutput, ParasimRefineError> {
	// The para id: from the authorizer config's `Vec<ParaId>` prefix (the real
	// service's layout), else the fallback for the null authorizer.
	let para_id = work_package_para_id(item_index).unwrap_or(FALLBACK_PARA_ID);

	let mut input: &[u8] = &payload.0;
	let candidate =
		parachain_service_interface::candidate::ParachainCandidate::decode_all(&mut input)
			.map_err(|_| ParasimRefineError::MalformedPayload)?;
	let head_data = pov::decode_para_head(&candidate.pov)
		.map_err(|e| match e {
			pov::PoVError::Compressed => ParasimRefineError::CompressedPoV,
			pov::PoVError::Malformed => ParasimRefineError::MalformedPoV,
		})?
		.try_into()
		.map_err(|_| ParasimRefineError::HeadTooLarge)?;

	Ok(ParasimWorkOutput { para_id, head_data })
}

/// The `ParaId` for `item_index` from the package's authorizer config, if the
/// config decodes as a `Vec<ParaId>` with an entry for that item.
fn work_package_para_id(item_index: usize) -> Option<ParaId> {
	let config = jam_pvm_common::refine::work_package().authorizer.config;
	let para_ids = Vec::<ParaId>::decode(&mut &config[..]).ok()?;
	para_ids.get(item_index).copied()
}

/// Upsert the head for one accumulated work item.
fn accumulate_one(result: &WorkOutput) {
	let mut input: &[u8] = &result.0;
	let Ok(output) = ParasimWorkOutput::decode_all(&mut input) else {
		// A stray/incompatible refine result should never wedge accumulate.
		return;
	};
	let key = para_head_key(output.para_id);
	let info = ParaInfoLite { head_data: output.head_data };
	if jam_pvm_common::accumulate::set_storage(&key, &info.encode()).is_err() {
		jam_pvm_common::error!("parasim: set_storage failed for para {:?}", output.para_id);
	}
}

/// The storage key of a para's head: tag `0x00` + SCALE(`ParaId`) — the real
/// service's `parachains` map layout (`Tag::Parachains`).
pub fn para_head_key(para_id: ParaId) -> Vec<u8> {
	let mut key = Vec::with_capacity(1 + para_id.encoded_size());
	key.push(0x00);
	para_id.encode_to(&mut key);
	key
}

/// A `ParaInfo` whose only meaningful field is `head_data`; SCALE layout matches
/// `parachain_service::state::para_info::ParaInfo` byte-for-byte (the collator
/// decodes the stored value as the real type). Kept local so parasim does not
/// depend on the churning service crate; the byte-compat test pins the equality.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ParaInfoLite {
	pub head_data: HeadData,
}

/// Structured reason `refine` failed to produce a head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ParasimRefineError {
	/// The work-item payload is not a decodable `ParachainCandidate`.
	MalformedPayload,
	/// The PoV is zstd-compressed; parasim does not decompress (the collator
	/// must skip compression in phases 1–2).
	CompressedPoV,
	/// The PoV is not a parseable `ParachainBlockData`.
	MalformedPoV,
	/// The extracted head exceeds `MAX_HEAD_DATA_SIZE`.
	HeadTooLarge,
}