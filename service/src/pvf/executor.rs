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
use alloc::{string::String, vec::Vec};
use codec::{DecodeAll, Encode};
use jam_pvm_common::refine;
use jam_types::{Hash, PageMode};
use parachain_service_core::{
	host_call::HostCall,
	types::ParaId,
	upward_message::{
		UpwardMessage, UpwardMessages, MAX_UPWARD_MESSAGE_BYTES, SET_VALIDATOR_KEYS_MAX_KEYS,
	},
};
use polkavm::Reg;

/// Returned in `A0` by buffer-returning calls when the value does not exist.
pub const ABSENT: u64 = u64::MAX;

/// Caps on what one `log` host call may copy out of the child, so a malformed guest
/// cannot force an unbounded peek on a diagnostics path.
const MAX_LOG_TARGET: u64 = 128;
const MAX_LOG_MESSAGE: u64 = 4096;

const A0: usize = Reg::A0 as usize;
const A1: usize = Reg::A1 as usize;
const A2: usize = Reg::A2 as usize;
const A3: usize = Reg::A3 as usize;
const A4: usize = Reg::A4 as usize;
const A5: usize = Reg::A5 as usize;

/// The child PVF's heap: a byte break plus the pages backing it.
///
/// The service drives the inner PVM's memory itself (`pages` host calls), so it tracks
/// the heap the guest grows, mirroring JAM's own `grow_heap` bounds in gp-v0.8.0
/// (host `grow_heap` at `crates/node/src/chain/exec/vm/host.rs`): the break starts at
/// the heap base `a` and may grow up to the address-space limit `b`
/// (`heap_base + max_heap_size`).
pub struct Heap {
	/// The page size of the inner PVM's memory map.
	pub page_size: u64,
	/// The current break: the byte end of the region the guest has grown to.
	pub top: u64,
	/// The byte end of the pages mapped (zeroed) so far; always page-aligned.
	pub mapped_until: u64,
	/// The address-space limit `b` = `heap_base + max_heap_size`.
	pub limit: u64,
}

impl Heap {
	/// Lay out the heap of the inner PVM `pvm.rs` set up: the break starts at the
	/// memory map's heap base, the pages up to the end of the initialised RW data are
	/// already mapped, and growth is bounded by the address-space limit `b`.
	pub fn new(memory: &polkavm::MemoryMap) -> Self {
		Self {
			page_size: u64::from(memory.page_size()),
			top: u64::from(memory.heap_base()),
			// `rw_data_address + rw_data_size` is the page-aligned end of the RW mapping
			// `pvm.rs` set up; the heap starts (possibly mid-page) at `heap_base` and
			// grows upward into the "heap slack" the builder leaves before the stack.
			mapped_until: u64::from(memory.rw_data_address()) + u64::from(memory.rw_data_size()),
			limit: u64::from(memory.heap_base()) + u64::from(memory.max_heap_size()),
		}
	}

	/// The new break after growing by `delta` bytes, or `None` if it would overflow or
	/// exceed the address-space limit `b` (gp-v0.8.0's `pages > address_space_limit`
	/// refusal, in byte terms).
	fn grow_to(&self, delta: u64) -> Option<u64> {
		let new_top = self.top.checked_add(delta)?;
		(new_top <= self.limit).then_some(new_top)
	}
}

/// The `(page, count, end)` range of pages in `[mapped_until, new_top)` that are not yet
/// mapped and must be zero-mapped to back the growth; `None` if `new_top` is already
/// within the mapped pages.
///
/// Whole pages are mapped, including the one containing `new_top`: the guest's allocator
/// only writes past the old break after the grow returns, so the over-mapped tail is
/// fresh, and `mapped_until` only ever advances, so no page is ever re-zeroed.
pub fn fresh_pages(mapped_until: u64, new_top: u64, page_size: u64) -> Option<(u64, u64, u64)> {
	let end = new_top.div_ceil(page_size) * page_size;
	(end > mapped_until).then(|| (mapped_until / page_size, (end - mapped_until) / page_size, end))
}

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
	/// The child's heap: break tracking + on-demand page mapping for `grow_heap`.
	heap: Heap,
}

impl ExecutorState {
	pub fn new(para_id: ParaId, heap: Heap) -> Self {
		Self {
			para_id,
			umps: UpwardMessages::new(),
			parent_head_hash: None,
			head_data: None,
			set_validator_keys_called: false,
			umps_bytes: 0,
			heap,
		}
	}

	/// The child's heap state (break, mapped boundary, limit) — public for the
	/// `grow_heap` contract tests in the blob test crate.
	pub fn heap(&self) -> &Heap {
		&self.heap
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
			panic!("PVF invoked unknown host call {index}; §4.2 whole-refine failure")
		};
		jam_pvm_common::info!(
			"PVF dispatch probe: call={} a0={:#x} a1={:#x}",
			index,
			regs[A0],
			regs[A1]
		);

		match call {
			// --- Data access (§4.3) -------------------------------------------------
			HostCall::Gas => {
				regs[A0] = refine::gas();
			},
			HostCall::GrowHeap => {
				// Child heap growth (§4.3): the guest allocator (sp-io's riscv
				// `global_alloc_riscv.rs`) calls `grow_heap(delta: usize) -> usize` — a
				// byte delta returning the *previous* break, or `0` on failure, and
				// `delta == 0` queries the current break. The service tracks the break
				// and maps the pages backing it into the inner PVM on demand, mirroring
				// gp-v0.8.0's host `grow_heap` bounds (break `h` vs limit `b`).
				let delta = regs[A0];
				if delta == 0 {
					regs[A0] = self.heap.top;
				} else {
					let Some(new_top) = self.heap.grow_to(delta) else {
						// Would overflow or exceed the address-space limit `b`: refuse.
						regs[A0] = 0;
						return Ok(());
					};
					if let Some((page, count, end)) =
						fresh_pages(self.heap.mapped_until, new_top, self.heap.page_size)
					{
						refine::zero(handle, page, count, PageMode::ReadWrite).unwrap_or_else(
							|_| {
								panic!("PVF `grow_heap` page mapping failed; §4.2 whole-refine failure")
							},
						);
						self.heap.mapped_until = end;
					}
					let old_top = self.heap.top;
					self.heap.top = new_top;
					regs[A0] = old_top;
					jam_pvm_common::info!(
						"PVF grow_heap probe: delta={delta} break={old_top:#x}->{new_top:#x} mapped={:#x}",
						self.heap.mapped_until
					);
				}
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
				jam_pvm_common::info!(
					"PVF fetch probe: kind={} a={} b={} full={} cap={}",
					regs[A3],
					regs[A4],
					regs[A5],
					full,
					cap
				);
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
			HostCall::Log => {
				// Diagnostics only (§4.3): no state, no digest, no bearing on the
				// result. This is how a guest panic leaves a message — `sp_io`'s riscv
				// panic handler formats the `PanicInfo` and sends it here, so an
				// assertion inside `jam_validate_block` shows up as text instead of a
				// bare trap at some program counter.
				let target = peek_bytes(handle, regs[A1], regs[A2].min(MAX_LOG_TARGET));
				let message = peek_bytes(handle, regs[A3], regs[A4].min(MAX_LOG_MESSAGE));
				let target = String::from_utf8_lossy(&target);
				let message = String::from_utf8_lossy(&message);
				match regs[A0] {
					0 => jam_pvm_common::error!("PVF [{target}] {message}"),
					1 => jam_pvm_common::warn!("PVF [{target}] {message}"),
					2 => jam_pvm_common::info!("PVF [{target}] {message}"),
					3 => jam_pvm_common::debug!("PVF [{target}] {message}"),
					_ => jam_pvm_common::trace!("PVF [{target}] {message}"),
				}
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
