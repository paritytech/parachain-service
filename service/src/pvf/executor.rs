//! PVM executor state for the PVF refine invocation: dispatches every child
//! host call (spec §4.3), buffering side effects as upward messages and
//! forwarding data-access calls to the outer JAM refine host calls.
//!
//! # Child host-call ABI (DECISIONS.md D-1)
//!
//! Arguments are passed in `A0..A5`. Pointers are guest addresses; the host
//! peeks/pokes through the `machine` handle.
//!
//! Buffer-returning data-access calls take a trailing `(out_ptr, out_cap)`
//! pair. The host writes `min(len, out_cap)` bytes to `out_ptr` and returns the
//! full length in `A0`, or [`ABSENT`] when the requested value does not exist —
//! a guest seeing `len > out_cap` retries with a larger buffer.
//!
//! Side-effect calls return nothing; they either succeed, fail with a
//! structured [`RefineLog`] error, or — on an abnormal PVF exit — panic the
//! whole Refine invocation (§4.2).

use crate::{
	constants::AUTHORIZER_QUEUE_LEN,
	work_digest::{HeadData, RefineLog, MAX_REPORT_ERROR_PAYLOAD},
};
use alloc::vec::Vec;
use codec::{DecodeAll, Encode};
use jam_pvm_common::refine;
use jam_types::Hash;
use parachain_service_interface::{
	host_call::HostCall,
	types::ParaId,
	upward_message::{
		UpwardMessage, UpwardMessages, MAX_UPWARD_MESSAGE_BYTES, SET_VALIDATOR_KEYS_MAX_KEYS,
	},
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
	/// `SetValidatorKeys` may be sent at most once per Refine (§4.3).
	set_validator_keys_called: bool,
	/// Running encoded size of `umps`, against the §4.3 budget.
	umps_bytes: usize,
}

impl ExecutorState {
	pub fn new(para_id: ParaId) -> Self {
		Self {
			para_id,
			umps: UpwardMessages::new(),
			parent_head_hash: None,
			head_data: None,
			set_validator_keys_called: false,
			umps_bytes: 0,
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
	///
	/// Structured violations return a `RefineLog`; abnormal PVF exits (unknown
	/// host calls, machine failures, oversized values) panic the whole refine
	/// invocation (§4.2).
	pub fn dispatch(
		&mut self,
		handle: u64,
		index: u64,
		regs: &mut [u64; 13],
	) -> Result<(), RefineLog> {
		let Ok(call) = HostCall::try_from(index) else {
			// Unknown host-call index: the PVF is malformed and fails the whole
			// refine invocation (§4.2).
			panic!("PVF invoked unknown host call {index}; §4.2 whole-refine failure");
		};

		match call {
			// --- Data access (§4.3) -------------------------------------------------
			HostCall::Gas => {
				regs[A0] = refine::gas();
			},
			HostCall::GrowHeap => {
				// The child's RW region is the inner PVM this service drives, not
				// JAM's own, so JAM's `grow_heap` is not what the child needs and
				// cannot be relayed.
				// FIXME: give the child a heap-growth path, or drop the index from §4.3.
				panic!("PVF `grow_heap` is not relayable; §4.2 whole-refine failure");
			},
			HostCall::Fetch => {
				// Forwarded unchanged (§4.3): the child's `(kind, a, b)` go straight
				// to JAM, so the service never interprets them. JAM writes into the
				// service's own memory, so the result is relayed into the child after.
				let (out_ptr, offset, cap) = (regs[A0], regs[A1], regs[A2]);
				let mut buf = alloc::vec![0u8; cap as usize];
				let full = unsafe {
					jam_pvm_common::imports::fetch(
						buf.as_mut_ptr(),
						offset,
						cap,
						regs[A3],
						regs[A4],
						regs[A5],
					)
				};
				relay_out(handle, full, &buf, out_ptr, cap, "fetch");
				regs[A0] = full;
			},
			HostCall::HistoricalLookup => {
				// Serves both own and foreign lookups; `service == u64::MAX` is JAM's
				// self sentinel, passed through untouched.
				let (service, hash_ptr, out_ptr) = (regs[A0], regs[A1], regs[A2]);
				let (offset, cap) = (regs[A3], regs[A4]);
				let hash = peek_hash(handle, hash_ptr);
				let mut buf = alloc::vec![0u8; cap as usize];
				let full = unsafe {
					jam_pvm_common::imports::historical_lookup(
						service,
						hash.as_ptr(),
						buf.as_mut_ptr(),
						offset,
						cap,
					)
				};
				relay_out(handle, full, &buf, out_ptr, cap, "historical_lookup");
				regs[A0] = full;
			},

			// --- Side effects (§4.3) -------------------------------------------------
			HostCall::Export => {
				let data = peek_bytes(handle, regs[A0], regs[A1]);
				let index = refine::export_slice(&data)
					.unwrap_or_else(|_| panic!("PVF `export` failed; §4.2 whole-refine failure"));
				regs[A0] = index;
			},
			HostCall::SetParentHeadHash => {
				// Mandatory exactly once; a second call makes the invocation
				// invalid, same as never calling it (§4.2).
				if self.parent_head_hash.is_some() {
					return Err(RefineLog::MissingHeadDeclaration);
				}
				self.parent_head_hash = Some(peek_hash(handle, regs[A0]));
			},
			HostCall::SetHead => {
				if self.head_data.is_some() {
					return Err(RefineLog::MissingHeadDeclaration);
				}
				let bytes = peek_bytes(handle, regs[A0], regs[A1]);
				// §4.3: an oversized head fails this digest, not the whole
				// invocation — the parachain gets a log entry it can act on.
				self.head_data =
					Some(HeadData::try_from(bytes).map_err(|_| RefineLog::HeadDataTooLarge)?);
			},
			HostCall::SendUpwardMessage => {
				// §4.3: one host call now carries the whole `UpwardMessage`
				// vocabulary as a SCALE blob. A message that fails to decode is a
				// malformed PVF, not a digest-level error, so it panics (§4.2).
				let encoded = peek_bytes(handle, regs[A0], regs[A1]);
				let msg = UpwardMessage::decode_all(&mut &encoded[..]).unwrap_or_else(|_| {
					panic!("PVF `send_upward_message` payload did not decode; §4.2 whole-refine failure")
				});
				self.push(msg)?;
			},
			HostCall::ReportError => {
				// Abort the PVF, failing Refine with the opaque payload; bytes
				// beyond the cap are truncated (§4.3).
				let len = regs[A1].min(MAX_REPORT_ERROR_PAYLOAD as u64);
				let payload = peek_bytes(handle, regs[A0], len);
				return Err(RefineLog::Opaque(
					payload.try_into().expect("truncated to the bound; qed"),
				));
			},
		}
		Ok(())
	}

	/// Buffer an upward message, applying every §4.3 rule the per-message host
	/// calls used to enforce individually: the parachain restrictions, the
	/// requirements documented on each variant, the message-count cap, and the
	/// parachain's encoded-message budget.
	fn push(&mut self, msg: UpwardMessage) -> Result<(), RefineLog> {
		if !msg.allowed_for(self.para_id) {
			return Err(RefineLog::RestrictedHostFunction);
		}
		match &msg {
			UpwardMessage::AssignCore { queue, new_assigner, .. } => {
				// A handoff cannot re-present a short queue afterwards, so it
				// requires exactly `AUTHORIZER_QUEUE_LEN` hashes (§4.3, §7.1).
				let len_ok = (1..=AUTHORIZER_QUEUE_LEN).contains(&queue.len());
				let handoff_ok = new_assigner.is_none() || queue.len() == AUTHORIZER_QUEUE_LEN;
				if !len_ok || !handoff_ok {
					return Err(RefineLog::InvalidAuthorizerQueue);
				}
			},
			UpwardMessage::SetValidatorKeys { keys, .. } => {
				if keys.len() > SET_VALIDATOR_KEYS_MAX_KEYS {
					return Err(RefineLog::TooManyValidatorKeys);
				}
				if self.set_validator_keys_called {
					return Err(RefineLog::SetValidatorKeysRepeated);
				}
				self.set_validator_keys_called = true;
			},
			_ => {},
		}
		// §4.3: the budget counts the encoded messages alone, independently of
		// the Gray Paper's 48 KiB combined result-blob limit.
		let size = msg.encoded_size();
		if self.umps_bytes + size > MAX_UPWARD_MESSAGE_BYTES {
			return Err(RefineLog::UpwardMessagesTooLarge);
		}
		self.umps.try_push(msg).map_err(|_| RefineLog::TooManyUpwardMessages)?;
		self.umps_bytes += size;
		Ok(())
	}
}

fn peek_bytes(handle: u64, ptr: u64, len: u64) -> Vec<u8> {
	if len == 0 {
		return Vec::new();
	}
	refine::peek(handle, ptr, len)
		.unwrap_or_else(|_| panic!("PVF guest memory peek failed; §4.2 whole-refine failure"))
}

/// Copy a forwarded JAM result into the child's buffer. `full` is JAM's return
/// value: the untruncated length, or [`ABSENT`] when the item does not exist.
fn relay_out(handle: u64, full: u64, buf: &[u8], out_ptr: u64, cap: u64, what: &str) {
	if full == ABSENT || cap == 0 {
		return;
	}
	let n = full.min(cap) as usize;
	refine::poke(handle, &buf[..n], out_ptr)
		.unwrap_or_else(|_| panic!("PVF `{what}` copy-out poke failed; §4.2 whole-refine failure"));
}

fn peek_hash(handle: u64, ptr: u64) -> Hash {
	let bytes = peek_bytes(handle, ptr, 32);
	bytes.try_into().expect("peeked exactly 32 bytes; qed")
}
