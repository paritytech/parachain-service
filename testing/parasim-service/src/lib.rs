//! parasim — a real JAM service with fake logic (spec B.0 item 8).
//!
//! Accepts a parachain work package without running its PVF, extracts the new para head from the
//! payload (`ParachainBlockData`), and upserts it into this service's own key–value store under
//! the real `parachain-service` storage-key layout — tag `0x00` + SCALE(`ParaId`) → a byte-exact
//! `ParaInfo` whose only meaningful field is `head_data`. The collator code that reads the para
//! head via `serviceValue` carries over unchanged to the real service.
//!
//! The PoV itself is not validated, but the *ancestry* it claims is: every package must carry a
//! proof of the para's previous head at its anchor, and its first block must build on that head.
//! Without this a dropped package would be papered over by the next one instead of stalling the
//! para, so retry semantics would only appear to work.
//!
//! Deliberately NOT the real service: no PVF, no log, no kv storage semantics, no transfers, no
//! upgrade tracking, no §5.5 head commitment. Deliberately a separate crate (not a mode of
//! `parachain-service`, which is churning on the POC branch).

#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use codec::{Decode, DecodeAll, Encode};
use jam_pvm_common::{declare_service, Service};
use jam_state_helpers::StateProof;
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
		service_id: ServiceId,
		payload: WorkPayload,
		_package_hash: WorkPackageHash,
	) -> WorkOutput {
		match refine_inner(item_index, service_id, &payload) {
			Ok(output) => WorkOutput(output.encode()),
			Err(error) => {
				jam_pvm_common::error!(
					"parasim: refine failed: {error:?} payload len={} head={:02x?}",
					payload.0.len(),
					&payload.0[..payload.0.len().min(16)]
				);
				WorkOutput(error.encode())
			},
		}
	}

	fn accumulate(_slot: Slot, _id: ServiceId, item_count: usize) -> Option<Hash> {
		jam_pvm_common::error!("parasim: accumulate called item_count={item_count}");
		for item in jam_pvm_common::accumulate::accumulate_items() {
			if let jam_types::AccumulateItem::WorkItem(record) = item {
				match record.result {
					Ok(result) => {
						jam_pvm_common::error!(
							"parasim: accumulate result ok, result len={}",
							result.0.len()
						);
						accumulate_one(&result);
					},
					Err(e) => jam_pvm_common::error!("parasim: accumulate work-item Err: {e:?}"),
				}
			}
		}
		None
	}
}

/// Decode a work item, verify the ancestry it claims, and re-emit the new head.
fn refine_inner(
	item_index: usize,
	service_id: ServiceId,
	payload: &WorkPayload,
) -> Result<ParasimWorkOutput, ParasimRefineError> {
	// The para id: from the authorizer config's `Vec<ParaId>` prefix (the real
	// service's layout), else the fallback for the null authorizer.
	let para_id = work_package_para_id(item_index).unwrap_or(FALLBACK_PARA_ID);

	let mut input: &[u8] = &payload.0;
	let candidate =
		parachain_service_interface::candidate::ParachainCandidate::decode_all(&mut input)
			.map_err(|_| ParasimRefineError::MalformedPayload)?;
	let pov = pov::decode_pov(&candidate.pov).map_err(|error| match error {
		pov::PoVError::Compressed => ParasimRefineError::CompressedPoV,
		pov::PoVError::Malformed => ParasimRefineError::MalformedPoV,
		pov::PoVError::MissingProof => ParasimRefineError::MissingProof,
	})?;

	check_ancestry(service_id, para_id, &pov)?;

	let head_data =
		pov.head.to_vec().try_into().map_err(|_| ParasimRefineError::HeadTooLarge)?;
	Ok(ParasimWorkOutput { para_id, head_data })
}

/// Require that this PoV builds on the para head recorded in JAM state at the anchor.
///
/// This is what makes a dropped work package *stall* the para rather than be papered over: the
/// next package must still chain onto the head that is actually stored, so a gap cannot heal by
/// overwriting. Refine has no way to read state directly, so the previous head arrives as a proof
/// against `RefineContext::state_root` — a root JAM itself checks on-chain when the package is
/// reported, which is what makes trusting it here sound.
fn check_ancestry(
	service_id: ServiceId,
	para_id: ParaId,
	pov: &pov::PoV,
) -> Result<(), ParasimRefineError> {
	let (anchor_state_root, proof) =
		<([u8; 32], StateProof)>::decode_all(&mut &pov.anchor_state_proof[..])
			.map_err(|_| ParasimRefineError::MalformedProof)?;

	// The collator picks the anchor and proves against it; the two must be the same state, or the
	// proof says nothing about the state this package will be reported against.
	if anchor_state_root != *jam_pvm_common::refine::refine_context().state_root {
		return Err(ParasimRefineError::ProofNotAtAnchor);
	}

	let state_key =
		jam_state_helpers::service_value_state_key(service_id, &para_head_key(para_id));
	let stored = jam_state_helpers::verify(&proof, &anchor_state_root, &state_key)
		.map_err(|_| ParasimRefineError::InvalidProof)?;

	let Some(stored) = stored else {
		// Proven absent: nothing has been stored for this para, so this is its first block and
		// there is no parent to match.
		return Ok(());
	};

	let info =
		ParaInfoLite::decode_all(&mut &stored[..]).map_err(|_| ParasimRefineError::InvalidProof)?;
	// A substrate header's hash is the blake2b-256 of its encoding, and `parent_hash` is the
	// first field of the header the collator built on top of it.
	if pov.parent_hash != jam_state_helpers::blake2_256(&info.head_data) {
		return Err(ParasimRefineError::WrongParent);
	}
	Ok(())
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
	let info = ParaInfoLite::with_head(output.head_data);
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
/// decodes the stored value as the real type), so the real type's trailing
/// fields are carried explicitly with their default values. Kept local so
/// parasim does not depend on the churning service crate; the byte-compat test
/// pins the equality.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ParaInfoLite {
	pub head_data: HeadData,
	/// Always `None` (real field: `Option<ValidationCode>`).
	pub validation_code: Option<()>,
	/// Always `None` (real field: `Option<(ValidationCode, Timeslot)>`).
	pub pending_upgrade: Option<()>,
	#[codec(compact)]
	pub total_state_balance: u64,
	#[codec(compact)]
	pub used_state_balance: u64,
	pub is_deregistering: bool,
}

impl ParaInfoLite {
	pub fn with_head(head_data: HeadData) -> Self {
		Self {
			head_data,
			validation_code: None,
			pending_upgrade: None,
			total_state_balance: 0,
			used_state_balance: 0,
			is_deregistering: false,
		}
	}
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
	/// The PoV carries no `jam/anchor_state_proof` entry, so the previous head is unknowable.
	MissingProof,
	/// The proof entry is not a decodable `(state_root, StateProof)`.
	MalformedProof,
	/// The proof was built against a different state root than this package's anchor, so it
	/// proves nothing about the state the package will be reported against.
	ProofNotAtAnchor,
	/// The proof does not verify against the anchor state root.
	InvalidProof,
	/// The first block does not build on the para head recorded in JAM state.
	WrongParent,
}