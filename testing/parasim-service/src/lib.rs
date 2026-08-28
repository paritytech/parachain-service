//! parasim — a real JAM service with fake logic (spec B.0 item 8).
//!
//! Accepts a parachain work package without running its PVF, extracts the new para head from the
//! payload (`ParachainBlockData`), and upserts it into this service's own key–value store under
//! the real `parachain-service` storage-key layout — tag `0x00` + SCALE(`ParaId`) → a byte-exact
//! `ParaInfo` whose only meaningful field is `head_data`. The collator code that reads the para
//! head via `serviceValue` carries over unchanged to the real service.
//!
//! The PoV itself is not validated, but the *ancestry* it claims is. A package either builds on
//! the head proven to be in JAM state at its anchor, or — under pipelining — on a block that has
//! been refined but not yet accumulated, whose header it imports as segment 0 of its parent's
//! package. Refine exports the new head as its own segment 0 once, and only once, that check
//! passes. Accumulate then has the last word: it writes the head only if the package still builds
//! on the head that is stored. Without all this a dropped package would be papered over by the
//! next one instead of stalling the para, so retry semantics would only appear to work.
//!
//! Deliberately NOT the real service: no PVF, no log, no kv storage semantics, no transfers, no
//! upgrade tracking, no §5.5 head commitment. Deliberately a separate crate (not a mode of
//! `parachain-service`, which is churning on the POC branch).

#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;

use alloc::vec::Vec;
use codec::{Compact, Decode, DecodeAll, Encode};
use jam_pvm_common::{declare_service, Service};
use jam_state_helpers::StateProof;
use jam_types::{
	CoreIndex, Hash, ServiceId, Slot, WorkOutput, WorkPackageHash, WorkPayload, SEGMENT_LEN,
};
use parachain_service_interface::types::{HeadData, ParaId};

pub mod pov;

/// Directory of this crate's `Cargo.toml`, used by `parasim-service/bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Length of a substrate hash, and so of a head hash.
const HASH_LEN: usize = 32;

/// Longest SCALE compact-`u32` length prefix, so an exported segment is built in one allocation.
const MAX_LENGTH_PREFIX_LEN: usize = 5;

/// The `ParaId` parasim falls back to when the authorizer config does not pin
/// one. Phases 1–2 run under the null authorizer (empty config), which decodes
/// to no paras; without a fallback parasim would reject every package. Mock-only;
/// removed once a real authorizer config arrives in phase 3.
pub const FALLBACK_PARA_ID: ParaId = ParaId(0);

/// What parasim's `refine` hands to `accumulate` for one work item.
/// Accumulate cannot see the work-item payload, so the head (and the para it
/// belongs to) travel in the work output.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ParasimWorkOutput {
	pub para_id: ParaId,
	pub head_data: HeadData,
	/// blake2b-256 of the head this block was refined against. Accumulate compares it with the
	/// head actually stored, which is the only place the para's lineage is decided on-chain.
	pub parent_head_hash: [u8; HASH_LEN],
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

	let parent_head_hash = resolve_parent(service_id, para_id, &pov)?;

	let head_data =
		pov.head.to_vec().try_into().map_err(|_| ParasimRefineError::HeadTooLarge)?;
	// Export last: a child imports this segment as proof its parent was refined, so it must only
	// exist once every check above has passed. Returning early instead leaves the item's export
	// count short, and JAM then zeroes the exports and replaces this output with `BadExports`.
	export_head(pov.head)?;
	Ok(ParasimWorkOutput { para_id, head_data, parent_head_hash })
}

/// Publish the new head as segment 0, the only segment parasim exports.
fn export_head(head: &[u8]) -> Result<(), ParasimRefineError> {
	let mut segment = Vec::with_capacity(head.len() + MAX_LENGTH_PREFIX_LEN);
	head.encode_to(&mut segment);
	// A head is already bounded well below a segment, so this can only fire if that bound moves;
	// it is here so such a change surfaces as a parasim error rather than a bare host-call failure.
	if segment.len() > SEGMENT_LEN {
		return Err(ParasimRefineError::HeadTooLarge);
	}
	jam_pvm_common::refine::export_slice(&segment)
		.map(|_| ())
		.map_err(|_| ParasimRefineError::ExportFailed)
}

/// Establish which head this PoV builds on, and return that head's hash.
///
/// Under pipelining the parent is usually a block that has been refined but not yet accumulated,
/// so it is in no state parasim can read. It arrives instead as an imported segment exported by
/// the parent's own package — authenticated by JAM, which resolves and validates the import's
/// segment root on-chain when the package is reported.
fn resolve_parent(
	service_id: ServiceId,
	para_id: ParaId,
	pov: &pov::PoV,
) -> Result<[u8; HASH_LEN], ParasimRefineError> {
	let accumulated = proven_head(service_id, para_id, pov)?;
	if let Some(head) = &accumulated {
		let head_hash = jam_state_helpers::blake2_256(head);
		if pov.parent_hash == head_hash {
			return Ok(head_hash);
		}
	}

	let Some(segment) = jam_pvm_common::refine::import(0) else {
		// Nothing is stored for this para and nothing was imported: its first block, which has no
		// parent to match. Accumulate stays the authority — it accepts an unparented head only
		// while the store is still empty.
		return match accumulated {
			None => Ok(pov.parent_hash),
			Some(_) => Err(ParasimRefineError::MissingImport),
		};
	};
	// The convention is one segment, so a second one means the item was not built for parasim and
	// the extra segments are unaccounted for.
	if jam_pvm_common::refine::import(1).is_some() {
		return Err(ParasimRefineError::TooManyImports);
	}

	let header = imported_header(segment.as_slice())?;
	let header_hash = jam_state_helpers::blake2_256(header);
	if pov.parent_hash != header_hash {
		return Err(ParasimRefineError::ParentHashMismatch);
	}
	Ok(header_hash)
}

/// The parent header carried by segment 0 of the parent's package.
///
/// The segment is the SCALE length-prefixed header, zero-padded to `SEGMENT_LEN` on export.
/// Public so the collator side can pin the byte contract it has to build segments against.
pub fn imported_header(segment: &[u8]) -> Result<&[u8], ParasimRefineError> {
	let mut input = segment;
	let len = u32::from(
		Compact::<u32>::decode(&mut input)
			.map_err(|_| ParasimRefineError::UndecodableImportedHeader)?,
	) as usize;
	if len == 0 {
		// JAM replaces a *failed* item's exports with zero-segments and keeps the package valid,
		// so an all-zero segment is what a parent that never passed refine exports. Its hash is a
		// public constant, so treating it as an empty parent header would hand anyone a parent
		// they could name in a crafted block.
		return Err(ParasimRefineError::EmptyImportedHeader);
	}
	let header = input.get(..len).ok_or(ParasimRefineError::UndecodableImportedHeader)?;
	if !pov::is_header(header) {
		return Err(ParasimRefineError::UndecodableImportedHeader);
	}
	Ok(header)
}

/// The para head proven to be in JAM state at this package's anchor, or `None` if the proof shows
/// there is none.
///
/// Refine has no way to read state directly, so the head arrives as a proof against
/// `RefineContext::state_root` — a root JAM itself checks on-chain when the package is reported,
/// which is what makes trusting it here sound.
fn proven_head(
	service_id: ServiceId,
	para_id: ParaId,
	pov: &pov::PoV,
) -> Result<Option<Vec<u8>>, ParasimRefineError> {
	let (anchor_state_root, proof) =
		<([u8; HASH_LEN], StateProof)>::decode_all(&mut &pov.anchor_state_proof[..])
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
		return Ok(None);
	};
	let info =
		ParaInfoLite::decode_all(&mut &stored[..]).map_err(|_| ParasimRefineError::InvalidProof)?;
	Ok(Some(info.head_data.into_inner()))
}

/// The `ParaId` for `item_index` from the package's authorizer config, if the
/// config decodes as a `Vec<ParaId>` with an entry for that item.
fn work_package_para_id(item_index: usize) -> Option<ParaId> {
	let config = jam_pvm_common::refine::work_package().authorizer.config;
	let para_ids = Vec::<ParaId>::decode(&mut &config[..]).ok()?;
	para_ids.get(item_index).copied()
}

/// Upsert the head for one accumulated work item, if it still builds on the head that is stored.
fn accumulate_one(result: &WorkOutput) {
	let mut input: &[u8] = &result.0;
	let Ok(output) = ParasimWorkOutput::decode_all(&mut input) else {
		// A stray/incompatible refine result should never wedge accumulate.
		return;
	};
	let key = para_head_key(output.para_id);
	// Read the stored head per item rather than once for the call: a chain of packages can
	// accumulate together, and each one's parent is the head its predecessor just wrote.
	let stored = jam_pvm_common::accumulate::get_storage(&key);
	if !builds_on_stored_head(stored.as_deref(), &output.parent_head_hash) {
		jam_pvm_common::error!(
			"parasim: stale package for para {:?}: built on {:02x?}, stored head is {:02x?}",
			output.para_id,
			output.parent_head_hash,
			stored.as_deref().and_then(stored_head_hash),
		);
		return;
	}
	let info = ParaInfoLite::with_head(output.head_data);
	if jam_pvm_common::accumulate::set_storage(&key, &info.encode()).is_err() {
		jam_pvm_common::error!("parasim: set_storage failed for para {:?}", output.para_id);
	}
}

/// Whether a work item's parent is still the head the para is at.
///
/// The only on-chain authority on the para's lineage. Refine's checks run in-core against a parent
/// the chain has not agreed on yet, so a package refined against a stale or fabricated head has to
/// be caught here — and by then a sibling may already have taken the slot.
fn builds_on_stored_head(stored: Option<&[u8]>, parent_head_hash: &[u8; HASH_LEN]) -> bool {
	let Some(stored) = stored else {
		// Nothing stored: the para's first block, which has no parent to be fresh against.
		return true;
	};
	// Undecodable stored bytes are not "no head": overwriting them is exactly the papering-over
	// this check exists to prevent.
	stored_head_hash(stored) == Some(*parent_head_hash)
}

/// The hash of the head inside a stored `ParaInfo`.
fn stored_head_hash(stored: &[u8]) -> Option<[u8; HASH_LEN]> {
	let info = ParaInfoLite::decode_all(&mut &stored[..]).ok()?;
	Some(jam_state_helpers::blake2_256(&info.head_data))
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
	/// The block builds on neither the accumulated head nor any imported parent: nothing was
	/// imported, so there is no candidate parent to check it against.
	MissingImport,
	/// More than one segment was imported; parasim's convention is exactly one.
	TooManyImports,
	/// The imported segment's header is empty — what JAM exports for an item that *failed*
	/// refine, never a real parent.
	EmptyImportedHeader,
	/// The imported segment is not a length-prefixed substrate header.
	UndecodableImportedHeader,
	/// The block names a parent that is not the imported header it was submitted with.
	ParentHashMismatch,
	/// The new head could not be exported, so no child could ever chain onto this block.
	ExportFailed,
}
#[cfg(test)]
mod tests {
	use super::*;
	use alloc::vec;

	/// Encode a substrate `Header<u32>` with an empty digest.
	fn header_bytes(parent: [u8; HASH_LEN], number: u32) -> Vec<u8> {
		let mut header = parent.to_vec();
		Compact::from(number).encode_to(&mut header);
		header.extend_from_slice(&[0u8; HASH_LEN]); // state_root
		header.extend_from_slice(&[0u8; HASH_LEN]); // extrinsics_root
		header.push(0); // empty Digest
		header
	}

	/// What the host writes when `export_head` exports `head`: the length-prefixed header,
	/// zero-padded to a full segment.
	fn exported_segment(head: &[u8]) -> Vec<u8> {
		let mut segment = head.encode();
		segment.resize(SEGMENT_LEN, 0);
		segment
	}

	/// The byte contract between a parent's export and its child's import: whatever `export_head`
	/// writes, `imported_header` must hand back unchanged, padding and all.
	#[test]
	fn exported_head_round_trips_works() {
		let head = header_bytes([4u8; HASH_LEN], 7);
		assert_eq!(imported_header(&exported_segment(&head)), Ok(&head[..]));
	}

	/// The guard the phase turns on: a failed parent's export is a zero-segment, which reads as a
	/// zero-length header whose hash is a constant anyone can name as their parent.
	#[test]
	fn zero_segment_errors() {
		assert_eq!(
			imported_header(&[0u8; SEGMENT_LEN]),
			Err(ParasimRefineError::EmptyImportedHeader)
		);
	}

	#[test]
	fn undecodable_imported_header_errors() {
		// Length prefix present, contents not a header.
		let mut segment = vec![0xffu8; 40].encode();
		segment.resize(SEGMENT_LEN, 0);
		assert_eq!(
			imported_header(&segment),
			Err(ParasimRefineError::UndecodableImportedHeader)
		);

		// A prefix claiming more bytes than the segment holds.
		let mut truncated = Compact::from(SEGMENT_LEN as u32).encode();
		truncated.resize(SEGMENT_LEN, 0);
		assert_eq!(
			imported_header(&truncated),
			Err(ParasimRefineError::UndecodableImportedHeader)
		);

		// A header with the padding folded into its declared length: the walker must not stop
		// early and call the trailing zeroes part of the header.
		let head = header_bytes([5u8; HASH_LEN], 1);
		let mut padded = head.clone();
		padded.extend_from_slice(&[0u8; 8]);
		let mut segment = padded.encode();
		segment.resize(SEGMENT_LEN, 0);
		assert_eq!(
			imported_header(&segment),
			Err(ParasimRefineError::UndecodableImportedHeader)
		);
	}

	/// A head at the interface's maximum still fits a segment with its length prefix, which is
	/// what lets refine export it at all.
	#[test]
	fn maximal_head_fits_a_segment_works() {
		let max = parachain_service_interface::types::MAX_HEAD_DATA_SIZE as usize;
		let head = vec![0u8; max];
		assert!(head.encode().len() <= SEGMENT_LEN);
	}

	#[test]
	fn freshness_check_works() {
		let head = header_bytes([1u8; HASH_LEN], 1);
		let stored = ParaInfoLite::with_head(head.clone().try_into().expect("fits; qed")).encode();
		let head_hash = jam_state_helpers::blake2_256(&head);

		assert!(builds_on_stored_head(Some(&stored), &head_hash));
		// A package refined against an older head must not overwrite the newer one.
		assert!(!builds_on_stored_head(Some(&stored), &[0u8; HASH_LEN]));
		// Nothing stored yet: the para's first block has no parent to be fresh against.
		assert!(builds_on_stored_head(None, &[0u8; HASH_LEN]));
		// Bytes that are not a `ParaInfo` are not an empty store.
		assert!(!builds_on_stored_head(Some(&[0xffu8; 3]), &[0u8; HASH_LEN]));
	}
}
