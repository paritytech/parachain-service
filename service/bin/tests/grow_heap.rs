//! `grow_heap` host-call contract tests.
//!
//! The child PVF's allocator (sp-io's riscv `global_alloc_riscv.rs`, import index 1)
//! calls `grow_heap(delta: usize) -> usize`: a byte delta returning the *previous*
//! heap break on success, `0` on failure, and `delta == 0` queries the current break.
//! The service's executor tracks the break and maps the backing pages on demand,
//! mirroring gp-v0.8.0's host `grow_heap` bounds. These tests pin that contract at the
//! `dispatch` level.
//!
//! The executor forwards its data-access host calls to JAM through `jam_pvm_common`
//! imports (`gas`, `peek`, `poke`, `pages`, …), which link only inside a guest build.
//! The shims below satisfy those symbols for this host test binary; each panics on
//! contact, so a test that strays into a path needing real inner-PVM I/O fails loudly
//! instead of silently passing. The paths under test never touch them.

use parachain_service::{
	constants::MAX_PVF_HEAP_SIZE,
	pvf::executor::{fresh_pages, ExecutorState, Heap},
};
use parachain_service_core::{host_call::HostCall, types::ParaId};

#[no_mangle]
pub extern "C" fn gas() -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `gas`")
}
#[no_mangle]
pub extern "C" fn peek(_: u64, _: *mut u8, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `peek`")
}
#[no_mangle]
pub extern "C" fn poke(_: u64, _: *const u8, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `poke`")
}
#[no_mangle]
pub extern "C" fn pages(_: u64, _: u64, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `pages`")
}
#[no_mangle]
pub extern "C" fn machine(_: *const u8, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `machine`")
}
#[allow(improper_ctypes_definitions)]
#[no_mangle]
pub extern "C" fn invoke(_: u64, _: *mut core::ffi::c_void) -> (u64, u64) {
	panic!("test-only shim: the grow_heap paths under test must not call `invoke`")
}
#[no_mangle]
pub extern "C" fn expunge(_: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `expunge`")
}
#[no_mangle]
pub extern "C" fn fetch(_: *mut u8, _: u64, _: u64, _: u64, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `fetch`")
}
#[no_mangle]
pub extern "C" fn historical_lookup(_: u64, _: *const u8, _: *mut u8, _: u64, _: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `historical_lookup`")
}
#[no_mangle]
pub extern "C" fn export(_: *const u8, _: u64) -> u64 {
	panic!("test-only shim: the grow_heap paths under test must not call `export`")
}

const PAGE: u64 = 4096;
/// `Reg::A0`'s index in the 13-register array — polkavm's register file orders
/// `ra, sp, t0..t2, s0, s1, a0..a5` (A0 = 7, not 0).
const A0: usize = 7;

/// A heap whose base, mapped boundary and limit match the real PVF's order of
/// magnitude: RW data maps up to `mapped_until`, the break starts below it.
fn heap(top: u64, mapped_until: u64, limit: u64) -> Heap {
	Heap { page_size: PAGE, top, mapped_until, limit }
}

fn grow(exe: &mut ExecutorState, delta: u64) -> u64 {
	let mut regs = [0u64; 13];
	regs[A0] = delta;
	exe.dispatch(0, HostCall::GrowHeap as u64, &mut regs).expect("ok");
	regs[A0]
}

/// `grow_heap(0)` queries the current break without growing — the guest allocator's
/// first call in `allocate_address_space`.
#[test]
fn grow_heap_zero_queries_the_break() {
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_4000, u64::MAX));
	assert_eq!(grow(&mut exe, 0), 0x10_0000, "reports the current break");
	assert_eq!(exe.heap().top, 0x10_0000, "a query never moves the break");
	assert_eq!(exe.heap().mapped_until, 0x10_4000, "a query never maps pages");
}

/// A growth that fits the already-mapped pages succeeds and returns the *previous*
/// break — any non-zero previous break is the guest's success signal, the "returns
/// at least the request" half of the contract in the byte-delta form the SDK runtime
/// (not polkajam's page-count form) uses.
#[test]
fn grow_heap_within_mapped_region_succeeds() {
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_8000, 0x20_0000));
	assert_eq!(grow(&mut exe, 0x3000), 0x10_0000, "returns the previous break");
	assert_eq!(exe.heap().top, 0x10_3000, "the break advanced by the delta");
	assert_eq!(exe.heap().mapped_until, 0x10_8000, "no new pages needed, none mapped");
}

/// A growth that would exceed the address-space limit `b` is refused: the handler
/// returns `0`, which the guest's `expand_memory_until` reads as allocation failure
/// rather than silently succeeding.
#[test]
fn grow_heap_beyond_limit_is_refused() {
	let mut exe = ExecutorState::new(ParaId(0), heap(0x10_0000, 0x10_4000, 0x10_5000));
	assert_eq!(grow(&mut exe, 0x6000), 0, "refused: new break would exceed the limit");
	assert_eq!(exe.heap().top, 0x10_0000, "the break is unchanged on refusal");
	assert_eq!(exe.heap().mapped_until, 0x10_4000, "no pages mapped on refusal");
}

/// A growth that would overflow the break is refused the same way.
#[test]
fn grow_heap_overflow_is_refused() {
	let mut exe = ExecutorState::new(ParaId(0), heap(u64::MAX - 1, u64::MAX, u64::MAX));
	assert_eq!(grow(&mut exe, 2), 0, "refused: the break would overflow");
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
