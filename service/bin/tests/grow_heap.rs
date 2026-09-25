//! `grow_heap` + `log` host-call contract tests.
//!
//! The child PVF's allocator (jam-pvm-common's picoalloc `System`, import index 1)
//! calls `grow_heap(pages: u64) -> u64` with an ABSOLUTE target page count —
//! `ceil((heap_base + size) / page_size)` — and treats the call as success iff
//! `grow_heap(pages) >= pages`
//! (`vendor/polkajam/crates/jam-pvm-common/src/mem.rs`). The service's executor
//! mirrors PolkaJam's host `grow_heap`
//! (`vendor/polkajam/crates/node/src/chain/exec/vm/host.rs`): a request at or below
//! the current top page count returns the current top unchanged; a request past the
//! address-space limit `b` returns the current top (`< pages`, so the allocator sees
//! failure); otherwise the heap is grown to `pages * page_size` and `pages` is
//! returned. These tests pin that contract at the `dispatch` level.
//!
//! `jam-pvm-common`'s panic handler logs through PolkaJam's non-GP `log` host call
//! (index 100, ABI `(level, target_ptr, target_len, text_ptr, text_len)`); the
//! executor routes it through the same body as the Parachain Service's `log`
//! (index 204). One test below pins that dispatch.
//!
//! The executor forwards its data-access host calls to JAM through `jam_pvm_common`
//! imports (`gas`, `peek`, `poke`, `pages`, …), which link only inside a guest build.
//! The shims below satisfy those symbols for this host test binary; each panics on
//! contact, so a test that strays into a path needing real inner-PVM I/O fails loudly
//! instead of silently passing. `pages` records its calls (the mapping the `grow_heap`
//! tests assert on), and `peek` serves the fixed buffers of the `log` test only.

use jam_types::{PageMode, PageOperation};
use parachain_service::{
	constants::MAX_PVF_HEAP_SIZE,
	pvf::executor::{fresh_pages, ExecutorState, Heap},
};
use parachain_service_core::{host_call::HostCall, types::ParaId};
use std::sync::Mutex;

/// `pages` host calls recorded by the shim: `(handle, page, count, operation)`.
static PAGES_CALLS: Mutex<Vec<(u64, u64, u64, u64)>> = Mutex::new(Vec::new());
/// `peek` host calls recorded by the shim: `(handle, guest address, length)`.
static PEEK_CALLS: Mutex<Vec<(u64, u64, u64)>> = Mutex::new(Vec::new());

/// Handle the `log` test dispatches with; only this handle may `peek`, so a stray
/// `peek` on the `grow_heap` paths still fails loudly.
const LOG_HANDLE: u64 = 0x100;
/// Guest addresses the `log` test's `peek` shim serves.
const TARGET_PTR: u64 = 0x1000;
const TEXT_PTR: u64 = 0x2000;

/// The `pages` calls the executor made for `handle`, draining them.
fn pages_calls(handle: u64) -> Vec<(u64, u64, u64, u64)> {
	let mut calls = PAGES_CALLS.lock().unwrap();
	let (mine, rest): (Vec<_>, Vec<_>) = calls.drain(..).partition(|c| c.0 == handle);
	*calls = rest;
	mine
}

/// The `peek` calls the executor made for `handle`, draining them.
fn peek_calls(handle: u64) -> Vec<(u64, u64, u64)> {
	let mut calls = PEEK_CALLS.lock().unwrap();
	let (mine, rest): (Vec<_>, Vec<_>) = calls.drain(..).partition(|c| c.0 == handle);
	*calls = rest;
	mine
}

#[no_mangle]
pub extern "C" fn gas() -> u64 {
	panic!("test-only shim: the paths under test must not call `gas`")
}
/// Serves the `log` test's fixed target/text buffers; any other caller is a bug.
#[no_mangle]
pub extern "C" fn peek(handle: u64, dst: *mut u8, src: u64, len: u64) -> u64 {
	if handle != LOG_HANDLE {
		panic!("test-only shim: only the log test may call `peek`")
	}
	PEEK_CALLS.lock().unwrap().push((handle, src, len));
	let data: &[u8] = match src {
		TARGET_PTR => b"panic_handler",
		TEXT_PTR => b"Panic handler called!",
		_ => panic!("test-only shim: unexpected peek address {src:#x}"),
	};
	let n = (len as usize).min(data.len());
	// SAFETY: `dst` points at the `vec![0; len]` the caller allocated for this copy.
	unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), dst, n) };
	0
}
#[no_mangle]
pub extern "C" fn poke(_: u64, _: *const u8, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the paths under test must not call `poke`")
}
/// Records the mapping request instead of mapping: that request is what the
/// `grow_heap` tests assert on.
#[no_mangle]
pub extern "C" fn pages(handle: u64, page: u64, count: u64, operation: u64) -> u64 {
	PAGES_CALLS.lock().unwrap().push((handle, page, count, operation));
	0
}
#[no_mangle]
pub extern "C" fn machine(_: *const u8, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the paths under test must not call `machine`")
}
#[allow(improper_ctypes_definitions)]
#[no_mangle]
pub extern "C" fn invoke(_: u64, _: *mut core::ffi::c_void) -> (u64, u64) {
	panic!("test-only shim: the paths under test must not call `invoke`")
}
#[no_mangle]
pub extern "C" fn expunge(_: u64) -> u64 {
	panic!("test-only shim: the paths under test must not call `expunge`")
}
#[no_mangle]
pub extern "C" fn fetch(_: *mut u8, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the paths under test must not call `fetch`")
}
#[no_mangle]
pub extern "C" fn historical_lookup(_: u64, _: *const u8, _: *mut u8, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the paths under test must not call `historical_lookup`")
}
#[no_mangle]
pub extern "C" fn export(_: *const u8, _: u64) -> u64 {
	panic!("test-only shim: the paths under test must not call `export`")
}

const PAGE: u64 = 4096;
/// `Reg::A0`'s index in the 13-register array — polkavm's register file orders
/// `ra, sp, t0..t2, s0, s1, a0..a5` (A0 = 7, not 0).
const A0: usize = 7;
const A1: usize = 8;
const A2: usize = 9;
const A3: usize = 10;
const A4: usize = 11;

/// A heap whose base, mapped boundary and limit match the real PVF's order of
/// magnitude: RW data maps up to `mapped_until`, the break starts below it.
fn heap(top: u64, mapped_until: u64, limit: u64) -> Heap {
	Heap { page_size: PAGE, top, mapped_until, limit }
}

fn grow(exe: &mut ExecutorState, handle: u64, pages: u64) -> u64 {
	let mut regs = [0u64; 13];
	regs[A0] = pages;
	exe.dispatch(handle, HostCall::GrowHeap as u64, &mut regs).expect("ok");
	regs[A0]
}

/// An absolute target page count above the current top grows the break to
/// `pages * page_size`, maps the fresh pages, and returns `pages` — the guest's
/// `grow_heap(pages) >= pages` success signal.
#[test]
fn grow_heap_to_target_pages_maps_and_returns_the_target() {
	const HANDLE: u64 = 1;
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_0000, 0x20_0000));
	assert_eq!(grow(&mut exe, HANDLE, 0x120), 0x120, "returns the requested page count");
	assert_eq!(exe.heap().top, 0x12_0000, "the break is the page-aligned end");
	assert_eq!(exe.heap().mapped_until, 0x12_0000, "mapping advanced to the new break");
	assert_eq!(
		pages_calls(HANDLE),
		vec![(HANDLE, 0x100, 0x20, u64::from(PageOperation::Alloc(PageMode::ReadWrite)))],
		"pages 0x100..0x120 are zero-mapped read-write"
	);
}

/// A request at or below the current top page count is a no-op returning the
/// current top: the guest's `>= pages` check still succeeds, and no page is mapped.
#[test]
fn grow_heap_at_or_below_current_pages_is_a_noop() {
	const HANDLE: u64 = 2;
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_4000, 0x20_0000));
	assert_eq!(grow(&mut exe, HANDLE, 0), 0x100, "0 is below the current top");
	assert_eq!(grow(&mut exe, HANDLE, 0x100), 0x100, "equal to the current top");
	assert_eq!(grow(&mut exe, HANDLE, 0x80), 0x100, "still below the current top");
	assert_eq!(exe.heap().top, 0x10_0000, "the break never moved");
	assert_eq!(exe.heap().mapped_until, 0x10_4000, "no page was mapped");
	assert!(pages_calls(HANDLE).is_empty());
}

/// A request past the address-space limit `b` is refused: the current top page
/// count is returned, which is always `< pages`, so the allocator sees failure.
#[test]
fn grow_heap_beyond_limit_is_refused_returning_current_pages() {
	const HANDLE: u64 = 3;
	// limit 0x10_5000 -> b = 0x105 pages (floor division, as in PolkaJam's host).
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_4000, 0x10_5000));
	assert_eq!(grow(&mut exe, HANDLE, 0x106), 0x100, "returns the current top, < requested");
	assert_eq!(exe.heap().top, 0x10_0000, "the break is unchanged on refusal");
	assert_eq!(exe.heap().mapped_until, 0x10_4000, "no page mapped on refusal");
	assert!(pages_calls(HANDLE).is_empty());
}

/// A huge request cannot overflow the page/byte conversion: it is simply past the
/// limit and refused.
#[test]
fn grow_heap_huge_request_is_refused() {
	const HANDLE: u64 = 4;
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_4000, u64::MAX));
	assert_eq!(grow(&mut exe, HANDLE, u64::MAX), 0x100, "refused: beyond the limit");
	assert_eq!(exe.heap().top, 0x10_0000, "the break is unchanged");
}

/// `jam-pvm-common`'s panic handler logs through PolkaJam's non-GP `log`
/// (index 100) with `(level, target_ptr, target_len, text_ptr, text_len)` in
/// `A0..A4`; index 100 must dispatch (not be rejected as unknown) and read the
/// same ABI as the Parachain Service's `log` (index 204).
#[test]
fn log_hostcalls_100_and_204_share_the_abi() {
	let target = b"panic_handler";
	let text = b"Panic handler called!";
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_4000, u64::MAX));

	for index in [100u64, HostCall::Log as u64] {
		let mut regs = [0u64; 13];
		regs[A0] = 1; // warn
		regs[A1] = TARGET_PTR;
		regs[A2] = target.len() as u64;
		regs[A3] = TEXT_PTR;
		regs[A4] = text.len() as u64;
		exe.dispatch(LOG_HANDLE, index, &mut regs)
			.expect("index 100/204 is a known log host call");
		assert_eq!(regs[A0], 1, "log has no return value; A0 is untouched");
	}

	assert_eq!(
		peek_calls(LOG_HANDLE),
		vec![
			(LOG_HANDLE, TARGET_PTR, target.len() as u64),
			(LOG_HANDLE, TEXT_PTR, text.len() as u64),
			(LOG_HANDLE, TARGET_PTR, target.len() as u64),
			(LOG_HANDLE, TEXT_PTR, text.len() as u64),
		],
		"both indices read (target_ptr, target_len) then (text_ptr, text_len)"
	);
}

/// §4.3: the child's heap stops at 1 GiB, however much address space the memory map
/// leaves for it.
#[test]
fn heap_limit_capped_at_one_gib_works() {
	let memory = polkavm::MemoryMapBuilder::new(PAGE as u32)
		.rw_data_size(PAGE as u32)
		.stack_size(PAGE as u32)
		.build()
		.expect("minimal layout is valid");
	assert!(u64::from(memory.max_heap_size()) > MAX_PVF_HEAP_SIZE);
	assert_eq!(Heap::new(&memory).limit, u64::from(memory.heap_base()) + MAX_PVF_HEAP_SIZE);
}

/// The fresh-page computation maps whole pages including the one containing the new
/// break, and never re-maps an already-mapped page.
#[test]
fn fresh_pages_maps_whole_pages_and_stops_at_mapped() {
	// Growth entirely within the mapped pages: nothing to map.
	assert_eq!(fresh_pages(0x10_4000, 0x10_4000, PAGE), None);
	assert_eq!(fresh_pages(0x10_4000, 0x10_3fff, PAGE), None);
	// Growth to a mid-page end maps the page containing it (the over-mapped tail):
	// pages 0x10_4..0x10_7 exclusive = 3 pages, end aligned to 0x10_7000.
	assert_eq!(fresh_pages(0x10_4000, 0x10_6abc, PAGE), Some((0x104, 3, 0x10_7000)));
	// A page-aligned end maps exactly the intervening pages.
	assert_eq!(fresh_pages(0x10_4000, 0x10_7000, PAGE), Some((0x104, 3, 0x10_7000)));
}
