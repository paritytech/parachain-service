//! Guest global allocator: a bump allocator over a fixed `static` arena.
//!
//! JAM's `jam_v1` instruction set has no `sbrk` (the linker refuses to emit it), so a
//! growable in-guest heap in the style of `picoalloc` / RFC-145 cannot be built. Instead
//! we reserve a fixed-size heap in the runtime's zero-initialised RW (BSS) section — which
//! the refine service maps writable when it sets up the inner PVM — and hand out
//! monotonically bumped, aligned slices.
//!
//! `dealloc` is a no-op: the inner PVM is single-use and its memory is reclaimed wholesale
//! by `expunge`, so there is nothing to free mid-run. Allocation is therefore monotonic and
//! bounded by [`ARENA_SIZE`]; exhausting it returns null, which routes into sp-io's OOM
//! handler (an abort).

use core::{
	alloc::{GlobalAlloc, Layout},
	ptr,
	sync::atomic::{AtomicUsize, Ordering},
};

/// Size of the fixed guest heap. Baked into the blob's RW-data size (the refine service
/// maps exactly this many bytes for the inner PVM); tune here if a runtime needs more
/// headroom for `validate_block`.
const ARENA_SIZE: usize = 16 * 1024 * 1024;

/// The heap backing store. 16-byte aligned so allocations up to that alignment need no
/// extra padding. Zero-initialised, so it lives in BSS and costs blob size only as a
/// length, not as bytes.
#[repr(align(16))]
struct Arena([u8; ARENA_SIZE]);

static mut ARENA: Arena = Arena([0; ARENA_SIZE]);

/// Bytes handed out so far. The guest is single-threaded, but an atomic keeps the
/// `GlobalAlloc` impl trivially `Sync` and correct regardless.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

struct BumpAllocator;

unsafe impl GlobalAlloc for BumpAllocator {
	unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
		let base = ptr::addr_of_mut!(ARENA) as usize;
		let align = layout.align();
		let size = layout.size();

		let mut cursor = CURSOR.load(Ordering::Relaxed);
		loop {
			// Align the next free absolute address, then express the new cursor relative to
			// the arena base. `base` is a small BSS address, so the alignment add cannot
			// overflow; the checks below guard against exhaustion and any wraparound.
			let aligned = (base + cursor + (align - 1)) & !(align - 1);
			let next = (aligned - base) + size;
			if next > ARENA_SIZE || next < cursor {
				return ptr::null_mut();
			}
			match CURSOR.compare_exchange_weak(cursor, next, Ordering::Relaxed, Ordering::Relaxed) {
				Ok(_) => return aligned as *mut u8,
				Err(actual) => cursor = actual,
			}
		}
	}

	unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
		// No-op; see module docs (single-use PVM, freed wholesale by `expunge`).
	}
}

#[global_allocator]
static ALLOCATOR: BumpAllocator = BumpAllocator;
