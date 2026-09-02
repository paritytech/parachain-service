//! parasim — a real JAM service with fake logic (spec B.0 item 8).
//!
//! Accepts a parachain work package without running its PVF, extracts the new para head from the
//! payload (`ParachainBlockData`), and upserts it into this service's own key–value store under
//! the real `parachain-service` storage-key layout — tag `0x00` + SCALE(`ParaId`) → a byte-exact
//! `ParaInfo` whose only meaningful field is `head_data`. The collator code that reads the para
//! head via `serviceValue` carries over unchanged to the real service.
//!
//! The PoV itself is not validated, but the *ancestry* it claims is — and accumulate is the only
//! authority on it. Refine declares the parent the block was built on and verifies the anchor-state
//! proof that carries the accumulated head, but it cannot reject a block for building on something
//! else: under pipelining the parent is usually a block that has been refined and not yet
//! accumulated, so refine has no way to see it. Accumulate applies a head only if its parent is the
//! head that is stored, and parks it in the reorder buffer (`buffer.rs`) if the parent is plausibly
//! still on its way. Without that a dropped package would be papered over by the next one instead
//! of stalling the para, so retry semantics would only appear to work.
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
	AuthQueue, AuthorizerHash, CoreIndex, Hash, ServiceId, Slot, WorkOutput,
	WorkPackageHash, WorkPayload,
};
use parachain_service_interface::{
	// Renamed: `jam_types::AuthTrace` is the opaque blob JAM carries an authorization output in;
	// this is what our own authorizer puts inside it.
	authorization::{AuthTrace as ControlTrace, Command, CONTROL_COMMAND_PREFIX},
	types::{HeadData, ParaId},
};

use buffer::{BufferedCandidate, HeadStore, Outcome, ReorderBuffer, StoredHead};

pub mod buffer;
pub mod pov;

/// Directory of this crate's `Cargo.toml`, used by `parasim-service/bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Length of a substrate hash, and so of a head hash.
const HASH_LEN: usize = 32;

/// Storage tag of the para-head map (the real service's `Tag::Parachains`).
const PARA_HEAD_TAG: u8 = 0x00;

/// Storage tag of the reorder buffer.
///
/// Deliberately outside the real service's `0x00..=0x08` tag range: the buffer is parasim-only
/// scratch state, and every tag in that range is a map the collator or a future version of this
/// service reads under the same key shape. FIXME: needs a tag agreed with `parachain-service` if
/// the reorder buffer ever becomes part of the spec.
const BUFFER_TAG: u8 = 0xf0;

/// What parasim's `refine` hands to `accumulate` for one work item.
///
/// Accumulate sees neither the work-item payload nor the package, so everything it acts on has to
/// travel through here — the new head, or the core-assignment command the item carried. They are
/// two variants of one type rather than two shapes of one struct so that accumulate cannot
/// confuse them: a command has no para and no lineage, and applying or parking it as a head would
/// corrupt whichever para happened to decode out of it.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub enum ParasimWorkOutput {
	Head(RefinedHead),
	Command(Command),
}

/// A parachain block's new head, and what accumulate needs to place it in the para's lineage.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct RefinedHead {
	pub para_id: ParaId,
	pub head_data: HeadData,
	/// blake2b-256 of the head this block was refined against. Accumulate compares it with the
	/// head actually stored, which is the only place the para's lineage is decided on-chain.
	pub parent_head_hash: [u8; HASH_LEN],
	/// The new head's block number, which is what bounds the reorder buffer to a plausible
	/// horizon. Refine reads it out of the header bytes it already holds.
	pub number: u32,
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

	fn accumulate(slot: Slot, id: ServiceId, item_count: usize) -> Option<Hash> {
		jam_pvm_common::error!("parasim: accumulate called item_count={item_count} slot={slot}");
		for item in jam_pvm_common::accumulate::accumulate_items() {
			if let jam_types::AccumulateItem::WorkItem(record) = item {
				match record.result {
					Ok(result) => {
						jam_pvm_common::error!(
							"parasim: accumulate result ok, result len={}",
							result.0.len()
						);
						accumulate_one(id, slot, &result);
					},
					Err(e) => jam_pvm_common::error!("parasim: accumulate work-item Err: {e:?}"),
				}
			}
		}
		None
	}
}

/// Decode a work item and re-emit whatever accumulate has to act on: the new head a parachain
/// block carries, or the core-assignment command a control payload carries.
fn refine_inner(
	item_index: usize,
	service_id: ServiceId,
	payload: &WorkPayload,
) -> Result<ParasimWorkOutput, ParasimRefineError> {
	// Before the para id, because a parked core has no para at all and a command is exactly what
	// gets sent to one.
	if let Some(command) = payload_command(sudo(), &payload.0)? {
		return Ok(ParasimWorkOutput::Command(command));
	}

	// The para id comes from the authorizer config's `Vec<ParaId>` prefix and nowhere else (the
	// real service's layout): whoever installed the core's queue decided which para may run on
	// it, and the submitter has no say.
	let para_id = work_package_para_id(item_index).ok_or(ParasimRefineError::NoParaId)?;

	let mut input: &[u8] = &payload.0;
	let candidate =
		parachain_service_interface::candidate::ParachainCandidate::decode_all(&mut input)
			.map_err(|_| ParasimRefineError::MalformedPayload)?;
	let pov = pov::decode_pov(&candidate.pov).map_err(|error| match error {
		pov::PoVError::Compressed => ParasimRefineError::CompressedPoV,
		pov::PoVError::Malformed => ParasimRefineError::MalformedPoV,
		pov::PoVError::MissingProof => ParasimRefineError::MissingProof,
	})?;
	log_parent_relationship(&pov, proven_head(service_id, para_id, &pov)?.as_deref());

	let head_data = pov.head.to_vec().try_into().map_err(|_| ParasimRefineError::HeadTooLarge)?;
	Ok(ParasimWorkOutput::Head(RefinedHead {
		para_id,
		head_data,
		parent_head_hash: pov.parent_hash,
		number: pov.number,
	}))
}

/// The command a work-item payload carries, or `None` if it carries a parachain block.
///
/// `sudo` is the authorizer's verdict, not the submitter's claim, which is what keeps the command
/// lane closed to ordinary packages. A command arriving without it is refused rather than read as
/// a block: it is either a mistake or an attempt to smuggle one past the authorizer, and both are
/// worth seeing in the log.
fn payload_command(sudo: bool, payload: &[u8]) -> Result<Option<Command>, ParasimRefineError> {
	let Some(encoded) = payload.strip_prefix(CONTROL_COMMAND_PREFIX) else { return Ok(None) };
	if !sudo {
		return Err(ParasimRefineError::CommandWithoutSudo);
	}
	Command::decode_all(&mut &encoded[..])
		.map(Some)
		.map_err(|_| ParasimRefineError::MalformedCommand)
}

/// Whether the authorizer admitted this package through its `sudo` lane.
///
/// The trace is all refine gets to see of the authorization, and only the authorizer can write
/// it. A trace that does not decode is not an error: the null authorizer every core starts out
/// with returns an empty one, and a package riding such a core simply has no privilege.
fn sudo() -> bool {
	ControlTrace::decode_all(&mut &jam_pvm_common::refine::auth_trace().0[..])
		.map_or(false, |trace| trace.sudo)
}

/// Fill a core's authorizer queue, which is how a core is both assigned and freed.
///
/// The only `assign` call parasim makes, and the assigner it passes is `me` — accumulate's own
/// service-id argument — never anything a caller could influence. `assign`'s third argument is
/// the core's *new* assigner, so any other value hands the core away for good, and a rejected
/// `assign` writes nothing to the chain: the mistake would surface only as a core that had
/// quietly stopped listening.
fn apply_command(me: ServiceId, command: Command) {
	let (core, authorizer) = match command {
		Command::Assign { para_id, core, authorizer } => {
			jam_pvm_common::error!(
				"parasim: assigning core {core} to para {para_id:?}, authorizer {authorizer:02x?}"
			);
			(core, authorizer)
		},
		Command::Free { core, parked_authorizer } => {
			jam_pvm_common::error!(
				"parasim: parking core {core}, authorizer {parked_authorizer:02x?}"
			);
			(core, parked_authorizer)
		},
	};
	if let Err(error) =
		jam_pvm_common::accumulate::assign(core, &AuthQueue::new(AuthorizerHash(authorizer)), me)
	{
		jam_pvm_common::error!(
			"parasim: assign for core {core} failed: {error:?} (service {me} is not its assigner)"
		);
	}
}

/// Record how the block's parent relates to the head proven at the anchor.
///
/// Refine cannot make a decision out of this: a block at depth two or more legitimately builds on
/// a head accumulate has not applied yet, so a mismatch is the normal pipelined case rather than
/// an error. It is still the cheapest way to tell a root-case block from a pipelined one when a
/// para stops advancing, which is why it is logged rather than dropped.
fn log_parent_relationship(pov: &pov::PoV, proven: Option<&[u8]>) {
	match proven {
		None => {
			jam_pvm_common::debug!(
				"parasim: refine: number={} parent={:02x?}, no head proven at the anchor",
				pov.number,
				pov.parent_hash,
			);
		},
		Some(head) => {
			let proven_hash = jam_state_helpers::blake2_256(head);
			jam_pvm_common::debug!(
				"parasim: refine: number={} parent={:02x?} proven head={:02x?} on_proven_head={}",
				pov.number,
				pov.parent_hash,
				proven_hash,
				pov.parent_hash == proven_hash,
			);
		},
	}
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

/// Apply one accumulated work item: run the command it carried, or place its head.
fn accumulate_one(me: ServiceId, slot: Slot, result: &WorkOutput) {
	let Ok(output) = ParasimWorkOutput::decode_all(&mut &result.0[..]) else {
		// A stray/incompatible refine result should never wedge accumulate.
		return;
	};
	let output = match output {
		// Commands never touch the head store or the reorder buffer; they have no para to belong
		// to and no lineage to be judged against.
		ParasimWorkOutput::Command(command) => return apply_command(me, command),
		ParasimWorkOutput::Head(head) => head,
	};
	let para_id = output.para_id;
	let buffer_key = buffer_key(para_id);
	let stored_buffer = jam_pvm_common::accumulate::get_storage(&buffer_key);
	let mut buffer = ReorderBuffer::decode_or_empty(stored_buffer.as_deref());
	let arriving = BufferedCandidate {
		parent_head_hash: output.parent_head_hash,
		head_data: output.head_data,
		number: output.number,
		arrived_slot: slot,
	};

	let outcome = buffer.accept(
		&mut ParaHeadStore { para_id, key: para_head_key(para_id) },
		arriving,
		slot,
		jam_types::epoch_period(),
	);

	log_outcome(para_id, &outcome);
	store_buffer(&buffer_key, &buffer, stored_buffer.as_deref());
	jam_pvm_common::error!("parasim: buffer depth for para {para_id:?}: {}", buffer.depth());
}

/// A para's head in this service's storage.
struct ParaHeadStore {
	para_id: ParaId,
	key: Vec<u8>,
}

impl HeadStore for ParaHeadStore {
	fn head(&self) -> StoredHead {
		StoredHead::read(jam_pvm_common::accumulate::get_storage(&self.key).as_deref())
	}

	fn set_head(&mut self, candidate: &BufferedCandidate) -> bool {
		let info = ParaInfoLite::with_head(candidate.head_data.clone());
		if jam_pvm_common::accumulate::set_storage(&self.key, &info.encode()).is_err() {
			jam_pvm_common::error!("parasim: set_storage failed for para {:?}", self.para_id);
			return false;
		}
		// Gas exhaustion rolls accumulation back to the most recent checkpoint rather than to its
		// start, so checkpointing here is what lets a long drain keep the heads it already applied
		// and finish the rest on a later invocation.
		jam_pvm_common::accumulate::checkpoint();
		true
	}
}

/// Persist the buffer, skipping the write when it did not change.
fn store_buffer(key: &[u8], buffer: &ReorderBuffer, stored: Option<&[u8]>) {
	if buffer.is_empty() {
		if stored.is_some() {
			jam_pvm_common::accumulate::remove_storage(key);
		}
		return;
	}
	let encoded = buffer.encode();
	if stored != Some(&encoded[..]) &&
		jam_pvm_common::accumulate::set_storage(key, &encoded).is_err()
	{
		jam_pvm_common::error!("parasim: buffer set_storage failed for key {key:02x?}");
	}
}

/// Log every decision the buffer made. A drop or eviction whose reason is not here is a bug.
fn log_outcome(para_id: ParaId, outcome: &Outcome) {
	for applied in &outcome.applied {
		jam_pvm_common::error!(
			"parasim: applied head for para {para_id:?}: number={} head={:02x?} parent={:02x?}",
			applied.number,
			applied.head_hash(),
			applied.parent_head_hash,
		);
	}
	if let Some(buffered) = &outcome.buffered {
		jam_pvm_common::error!(
			"parasim: buffered head for para {para_id:?}: number={} head={:02x?} parent={:02x?}",
			buffered.number,
			buffered.head_hash(),
			buffered.parent_head_hash,
		);
	}
	if let Some((dropped, reason)) = &outcome.dropped {
		jam_pvm_common::error!(
			"parasim: dropped head for para {para_id:?}: {reason:?} number={} head={:02x?} \
			 parent={:02x?}",
			dropped.number,
			dropped.head_hash(),
			dropped.parent_head_hash,
		);
	}
	for (evicted, reason) in &outcome.evicted {
		jam_pvm_common::error!(
			"parasim: evicted head for para {para_id:?}: {reason:?} number={} head={:02x?} \
			 parent={:02x?}",
			evicted.number,
			evicted.head_hash(),
			evicted.parent_head_hash,
		);
	}
}

/// The storage key of a para's head: tag `0x00` + SCALE(`ParaId`) — the real
/// service's `parachains` map layout (`Tag::Parachains`).
pub fn para_head_key(para_id: ParaId) -> Vec<u8> {
	storage_key(PARA_HEAD_TAG, para_id)
}

/// The storage key of a para's reorder buffer.
pub fn buffer_key(para_id: ParaId) -> Vec<u8> {
	storage_key(BUFFER_TAG, para_id)
}

/// `[tag] || SCALE(para_id)`, the real service's storage-key layout.
fn storage_key(tag: u8, para_id: ParaId) -> Vec<u8> {
	let mut key = Vec::with_capacity(1 + para_id.encoded_size());
	key.push(tag);
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
	/// The authorizer config pins no `ParaId` for this work item, so nothing says which para the
	/// package speaks for. Every authorizer parasim runs under carries the prefix; the null one
	/// does not, and packages on an unassigned core are refused rather than guessed at.
	NoParaId,
	/// The payload is marked as a core-assignment command, but the authorization did not come
	/// through the `sudo` lane, so this package may not carry one.
	CommandWithoutSudo,
	/// The payload is marked as a core-assignment command but does not decode as one.
	MalformedCommand,
}
#[cfg(test)]
mod tests {
	use super::*;
	use alloc::vec;
	use codec::Compact;

	/// Encode a substrate `Header<u32>` with an empty digest.
	fn header_bytes(parent: [u8; HASH_LEN], number: u32) -> Vec<u8> {
		let mut header = parent.to_vec();
		Compact::from(number).encode_to(&mut header);
		header.extend_from_slice(&[0u8; HASH_LEN]); // state_root
		header.extend_from_slice(&[0u8; HASH_LEN]); // extrinsics_root
		header.push(0); // empty Digest
		header
	}

	/// The buffer's rules are all measured against the stored head, so reading it wrong would
	/// silently change every one of them.
	#[test]
	fn stored_head_works() {
		let head = header_bytes([1u8; HASH_LEN], 7);
		let stored = ParaInfoLite::with_head(head.clone().try_into().expect("fits; qed")).encode();

		assert_eq!(
			StoredHead::read(Some(&stored)),
			StoredHead::At { hash: jam_state_helpers::blake2_256(&head), number: 7 }
		);
		// Nothing stored yet: the para's first block has no parent to be fresh against.
		assert_eq!(StoredHead::read(None), StoredHead::Empty);
		// Bytes that are not a `ParaInfo`, and a `ParaInfo` whose head is not a header, are both
		// something other than an empty store: applying over them is the papering-over the
		// freshness check exists to prevent.
		assert_eq!(StoredHead::read(Some(&[0xffu8; 3])), StoredHead::Unreadable);
		let junk = ParaInfoLite::with_head(vec![0xffu8; 40].try_into().expect("fits; qed"));
		assert_eq!(StoredHead::read(Some(&junk.encode())), StoredHead::Unreadable);
	}

	/// The head map and the buffer must never collide: they are keyed by the same `ParaId`, and
	/// the buffer's tag is the one thing in this service that is not the real layout.
	#[test]
	fn storage_keys_are_distinct_works() {
		assert_eq!(para_head_key(ParaId(3)), vec![0x00, 3, 0, 0, 0]);
		assert_eq!(buffer_key(ParaId(3)), vec![0xf0, 3, 0, 0, 0]);
	}

	fn command() -> Command {
		Command::Free { core: 1, parked_authorizer: [7u8; HASH_LEN] }
	}

	fn command_payload() -> Vec<u8> {
		let mut payload = CONTROL_COMMAND_PREFIX.to_vec();
		command().encode_to(&mut payload);
		payload
	}

	/// The prefix is the only thing that makes a payload a command, and `sudo` — the authorizer's
	/// verdict, which the submitter cannot forge — is the only thing that makes refine read it.
	/// Without the second half, any collator could assign any core by putting the right bytes in
	/// a block it was going to send anyway.
	#[test]
	fn a_command_needs_both_the_prefix_and_sudo_works() {
		assert_eq!(payload_command(true, &command_payload()), Ok(Some(command())));
		// The same command bytes on the ordinary collator lane do nothing at all.
		assert_eq!(
			payload_command(false, &command_payload()),
			Err(ParasimRefineError::CommandWithoutSudo)
		);
		// A parachain block is not a command even with the privilege to run one.
		assert_eq!(payload_command(true, &command().encode()), Ok(None));
		assert_eq!(
			payload_command(true, CONTROL_COMMAND_PREFIX),
			Err(ParasimRefineError::MalformedCommand)
		);
	}

	/// Accumulate matches on the variant, so a command can never reach the head store or the
	/// reorder buffer — it has no para to belong to. Pinned because the two used to be one
	/// output type told apart by whether it decoded.
	#[test]
	fn a_command_output_is_not_a_head_works() {
		let encoded = ParasimWorkOutput::Command(command()).encode();
		assert!(matches!(
			ParasimWorkOutput::decode_all(&mut &encoded[..]),
			Ok(ParasimWorkOutput::Command(_))
		));
		assert!(RefinedHead::decode_all(&mut &encoded[..]).is_err());
		// Nor is it a rejection: a control package that reached accumulate did not fail.
		assert!(ParasimRefineError::decode_all(&mut &encoded[..]).is_err());
	}
}
