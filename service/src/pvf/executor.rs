//! PVM executor state for the PVF refine invocation: dispatches every child
//! host call (spec §4.3), buffering side effects as upward messages and
//! forwarding data-access calls to the outer JAM refine host calls.
//!
//! # Child host-call ABI (DECISIONS.md D-1; SPEC_GAPS #6)
//!
//! Arguments are passed in `A0..A5`. Pointers are guest addresses; the host
//! peeks/pokes through the `machine` handle.
//!
//! Buffer-returning data-access calls take a trailing `(out_ptr, out_cap)`
//! pair. The host writes `min(len, out_cap)` bytes to `out_ptr` and returns the
//! full length in `A0`, or [`ABSENT`] when the requested value does not exist —
//! a guest seeing `len > out_cap` retries with a larger buffer.
//!
//! Side-effect calls return nothing; they either succeed or abort the whole
//! Refine invocation with the documented `RefineLog` error.

use crate::work_digest::{HeadData, RefineLog, ValidationCodeHash, MAX_REPORT_ERROR_PAYLOAD};
use alloc::vec::Vec;
use codec::DecodeAll;
use jam_codec::Encode as JamEncode;
use jam_pvm_common::refine;
use jam_types::Hash;
use parachain_service_interface::{
	host_call::HostCall,
	types::{ParaId, ServiceId, Timeslot},
	upward_message::{TransferOutArgs, UpwardMessage, UpwardMessages, SET_VALIDATOR_KEYS_MAX_KEYS},
};
use polkavm::Reg;

/// Returned in `A0` by buffer-returning calls when the value does not exist.
pub const ABSENT: u64 = u64::MAX;

const A0: usize = Reg::A0 as usize;
const A1: usize = Reg::A1 as usize;
const A2: usize = Reg::A2 as usize;
const A3: usize = Reg::A3 as usize;
const A4: usize = Reg::A4 as usize;
const A5: usize = Reg::A5 as usize;

/// Side-effect buffer during the refine invoke-PVM loop.
pub struct ExecutorState {
	/// The authoritative para this work item speaks for (§3.2). Restricted
	/// host functions are checked against it (§4.3, DECISIONS.md D-2).
	para_id: ParaId,
	/// Upward messages in emission order, replayed by Accumulate.
	umps: UpwardMessages,
	/// From the mandatory `set_parent_head_hash` (§4.2).
	parent_head_hash: Option<Hash>,
	/// From the mandatory `set_head` (§4.2).
	head_data: Option<HeadData>,
	/// `set_validator_keys` may be called at most once per Refine (§4.3).
	set_validator_keys_called: bool,
}

impl ExecutorState {
	pub fn new(para_id: ParaId) -> Self {
		Self {
			para_id,
			umps: UpwardMessages::new(),
			parent_head_hash: None,
			head_data: None,
			set_validator_keys_called: false,
		}
	}

	/// Consume the state after the PVF halted: both head declarations are
	/// mandatory exactly once (§4.2).
	pub fn finish(self) -> Result<(Hash, HeadData, UpwardMessages), RefineLog> {
		match (self.parent_head_hash, self.head_data) {
			(Some(parent), Some(head)) => Ok((parent, head, self.umps)),
			_ => Err(RefineLog::MissingHeadDeclaration),
		}
	}

	/// Handle one child host call. `regs` are the inner PVM's registers at the
	/// fault; return-value registers are updated in place.
	pub fn dispatch(
		&mut self,
		handle: u64,
		index: u64,
		regs: &mut [u64; 13],
	) -> Result<(), RefineLog> {
		let Ok(call) = HostCall::try_from(index) else {
			// Unknown host-call index: the PVF is malformed.
			return Err(RefineLog::ValidationFailed);
		};

		match call {
			// --- Data access (§4.3) -------------------------------------------------
			HostCall::Gas => {
				regs[A0] = refine::gas();
			},
			HostCall::Lookup => {
				let hash = peek_hash(handle, regs[A0])?;
				let data = refine::lookup(&hash);
				regs[A0] = copy_out(handle, data.as_deref(), regs[A1], regs[A2])?;
			},
			HostCall::ForeignLookup => {
				let service = regs[A0] as ServiceId;
				let hash = peek_hash(handle, regs[A1])?;
				let data = refine::foreign_lookup(service, &hash);
				regs[A0] = copy_out(handle, data.as_deref(), regs[A2], regs[A3])?;
			},
			HostCall::WorkPackage => {
				let encoded = JamEncode::encode(&refine::work_package());
				regs[A0] = copy_out(handle, Some(&encoded), regs[A0], regs[A1])?;
			},
			HostCall::WorkPackageContext => {
				let encoded = JamEncode::encode(&refine::refine_context());
				regs[A0] = copy_out(handle, Some(&encoded), regs[A0], regs[A1])?;
			},
			HostCall::AuthConfig => {
				// NOTE: deliberately read through the work package, not
				// `refine::auth_config()` — the vendored accessor double-decodes
				// the blob.
				let config = refine::work_package().authorizer.config;
				regs[A0] = copy_out(handle, Some(&config[..]), regs[A0], regs[A1])?;
			},
			HostCall::AuthToken => {
				let token = refine::work_package().authorization;
				regs[A0] = copy_out(handle, Some(&token[..]), regs[A0], regs[A1])?;
			},
			HostCall::WorkItemsSummary => {
				let encoded = JamEncode::encode(&refine::work_items_summary());
				regs[A0] = copy_out(handle, Some(&encoded), regs[A0], regs[A1])?;
			},
			HostCall::WorkItemSummary => {
				let summary = refine::work_item_summary(regs[A0] as usize);
				let encoded = summary.map(|s| JamEncode::encode(&s));
				regs[A0] = copy_out(handle, encoded.as_deref(), regs[A1], regs[A2])?;
			},
			HostCall::WorkItemPayload => {
				let payload = refine::work_item_payload(regs[A0] as usize);
				regs[A0] = copy_out(handle, payload.as_deref(), regs[A1], regs[A2])?;
			},
			HostCall::ImportSegment => {
				let segment = refine::import(regs[A0] as usize);
				regs[A0] = copy_out(handle, segment.as_ref().map(|s| &s[..]), regs[A1], regs[A2])?;
			},

			// --- Side effects (§4.3) -------------------------------------------------
			HostCall::Export => {
				let data = peek_bytes(handle, regs[A0], regs[A1])?;
				let index = refine::export_slice(&data).map_err(|_| RefineLog::ValidationFailed)?;
				regs[A0] = index;
			},
			HostCall::SetParentHeadHash => {
				// Mandatory exactly once; a second call makes the invocation
				// invalid, same as never calling it (§4.2).
				if self.parent_head_hash.is_some() {
					return Err(RefineLog::MissingHeadDeclaration);
				}
				self.parent_head_hash = Some(peek_hash(handle, regs[A0])?);
			},
			HostCall::SetHead => {
				if self.head_data.is_some() {
					return Err(RefineLog::MissingHeadDeclaration);
				}
				let bytes = peek_bytes(handle, regs[A0], regs[A1])?;
				self.head_data =
					Some(HeadData::try_from(bytes).map_err(|_| RefineLog::HeadDataTooLarge)?);
			},
			HostCall::RequestCodeUpgrade => {
				let hash = ValidationCodeHash(peek_hash(handle, regs[A0])?);
				self.push(UpwardMessage::RequestCodeUpgrade {
					hash,
					len: (regs[A1] as u32).into(),
				})?;
			},
			HostCall::Solicit => {
				let hash = peek_hash(handle, regs[A0])?;
				self.push(UpwardMessage::Solicit { hash, len: (regs[A1] as u32).into() })?;
			},
			HostCall::Forget => {
				let para_id = ParaId(regs[A0] as u32);
				let hash = peek_hash(handle, regs[A1])?;
				self.push(UpwardMessage::Forget { para_id, hash, len: (regs[A2] as u32).into() })?;
			},
			HostCall::KvSet => {
				let key = peek_bytes(handle, regs[A0], regs[A1])?;
				let value = peek_bytes(handle, regs[A2], regs[A3])?;
				self.push(UpwardMessage::SetKV { key, value })?;
			},
			HostCall::KvRemove => {
				let para_id = ParaId(regs[A0] as u32);
				let key = peek_bytes(handle, regs[A1], regs[A2])?;
				self.push(UpwardMessage::RemoveKV { para_id, key })?;
			},
			HostCall::TransferOut => {
				// Seven fields exceed the six-register window, so the guest hands
				// over a SCALE-encoded `TransferOutArgs` blob instead (D-10).
				let encoded = peek_bytes(handle, regs[A0], regs[A1])?;
				let args = TransferOutArgs::decode_all(&mut &encoded[..])
					.map_err(|_| RefineLog::MalformedPayload)?;
				self.push(UpwardMessage::TransferOut(args))?;
			},
			HostCall::AssignCore => {
				let core = regs[A0] as u16;
				let count = regs[A2] as usize;
				let new_assigner = (regs[A3] != 0).then_some(regs[A4] as ServiceId);
				let jam_slot = regs[A5] as Timeslot;
				// §4.3: an `assign_core` queue holds 1..=AUTHORIZER_QUEUE_LEN
				// hashes, and a handoff to a new assigner demands exactly
				// AUTHORIZER_QUEUE_LEN — the service can no longer re-present a
				// short queue after giving the core away.
				let queue_len_ok = count >= 1 && count <= crate::constants::AUTHORIZER_QUEUE_LEN;
				let handoff_ok =
					new_assigner.is_none() || count == crate::constants::AUTHORIZER_QUEUE_LEN;
				if !queue_len_ok || !handoff_ok {
					return Err(RefineLog::InvalidAuthorizerQueue);
				}
				let raw = peek_bytes(handle, regs[A1], (count * 32) as u64)?;
				let queue = raw
					.chunks_exact(32)
					.map(|c| c.try_into().expect("chunks_exact(32); qed"))
					.collect::<Vec<[u8; 32]>>();
				self.push(UpwardMessage::AssignCore { core, queue, new_assigner, jam_slot })?;
			},
			HostCall::SetValidatorKeys => {
				let count = regs[A1] as usize;
				// At most once per Refine, at most 30 keys per chunk (§4.3, §5.3).
				if self.set_validator_keys_called || count > SET_VALIDATOR_KEYS_MAX_KEYS {
					return Err(RefineLog::SetValidatorKeysTooManyKeys);
				}
				self.set_validator_keys_called = true;
				let raw = peek_bytes(handle, regs[A0], (count * 336) as u64)?;
				let keys = raw
					.chunks_exact(336)
					.map(|c| c.try_into().expect("chunks_exact(336); qed"))
					.collect::<Vec<[u8; 336]>>();
				let is_last = regs[A2] != 0;
				self.push(UpwardMessage::SetValidatorKeys { keys, is_last })?;
			},
			HostCall::ConsumeTransfersUpTo => {
				self.push(UpwardMessage::ConsumeTransfersUpTo(regs[A0] as Timeslot))?;
			},
			HostCall::ParachainServiceUpgrade => {
				let code_hash = peek_hash(handle, regs[A0])?;
				self.push(UpwardMessage::UpgradeService {
					code_hash,
					len: (regs[A1] as u32).into(),
					min_acc_gas: regs[A2],
					min_memo_gas: regs[A3],
				})?;
			},
			HostCall::ReportError => {
				// Abort the PVF, failing Refine with the opaque payload; bytes
				// beyond the cap are truncated (§4.3).
				let len = regs[A1].min(MAX_REPORT_ERROR_PAYLOAD as u64);
				let payload = peek_bytes(handle, regs[A0], len)?;
				return Err(RefineLog::Opaque(
					payload.try_into().expect("truncated to the bound; qed"),
				));
			},
			HostCall::ParachainSetHead => {
				let para_id = ParaId(regs[A0] as u32);
				let bytes = peek_bytes(handle, regs[A1], regs[A2])?;
				let new_head = bytes.try_into().map_err(|_| RefineLog::HeadDataTooLarge)?;
				self.push(UpwardMessage::ParachainSetHead { para_id, new_head })?;
			},
			HostCall::ParachainSetValidationCode => {
				let para_id = ParaId(regs[A0] as u32);
				let hash = ValidationCodeHash(peek_hash(handle, regs[A1])?);
				self.push(UpwardMessage::ParachainSetValidationCode {
					para_id,
					new_validation_code_hash: hash,
					new_validation_code_len: (regs[A2] as u32).into(),
				})?;
			},
			HostCall::ParachainCleanUp => {
				self.push(UpwardMessage::ParachainCleanUp(ParaId(regs[A0] as u32)))?;
			},
			HostCall::ParachainSetStateBalance => {
				let para_id = ParaId(regs[A0] as u32);
				self.push(UpwardMessage::ParachainSetStateBalance {
					para_id,
					new_total: regs[A1].into(),
				})?;
			},
		}
		Ok(())
	}

	/// Buffer an upward message, aborting on a parachain-restriction violation
	/// (§4.3) or overflow of the 1024-message cap.
	fn push(&mut self, msg: UpwardMessage) -> Result<(), RefineLog> {
		if !msg.allowed_for(self.para_id) {
			return Err(RefineLog::RestrictedHostFunction);
		}
		self.umps.try_push(msg).map_err(|_| RefineLog::TooManyUpwardMessages)
	}
}

/// Copy `data` into the guest at `out_ptr`, capped at `out_cap` bytes; returns
/// the value for `A0` (full length, or [`ABSENT`]).
fn copy_out(
	handle: u64,
	data: Option<&[u8]>,
	out_ptr: u64,
	out_cap: u64,
) -> Result<u64, RefineLog> {
	let Some(data) = data else { return Ok(ABSENT) };
	let n = (data.len() as u64).min(out_cap) as usize;
	if n > 0 {
		refine::poke(handle, &data[..n], out_ptr).map_err(|_| RefineLog::ValidationFailed)?;
	}
	Ok(data.len() as u64)
}

/// Read `len` bytes of guest memory.
fn peek_bytes(handle: u64, ptr: u64, len: u64) -> Result<Vec<u8>, RefineLog> {
	if len == 0 {
		return Ok(Vec::new());
	}
	refine::peek(handle, ptr, len).map_err(|_| RefineLog::ValidationFailed)
}

/// Read a 32-byte hash from guest memory.
fn peek_hash(handle: u64, ptr: u64) -> Result<Hash, RefineLog> {
	let bytes = peek_bytes(handle, ptr, 32)?;
	Ok(bytes.try_into().expect("peeked exactly 32 bytes; qed"))
}
