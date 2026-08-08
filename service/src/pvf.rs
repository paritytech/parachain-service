//! Parsing and running a parachain validation function (PVF) as an inner PVM.

use crate::work_digest::RefineLog;
use alloc::vec::Vec;
use jam_pvm_common::{refine, InvokeOutcome};
use jam_types::{PageMode, PAGE_SIZE};
use polkavm::Reg;

/// A parachain validation function parsed into a form ready to run as an inner PVM.
///
/// `machine` spawns the inner PVM code-only, so we also carry its memory image and layout
/// to set up before invoking.
pub struct ParsedPvf {
	/// `code + jump table` blob, as the `machine` host call expects it.
	pub code: polkavm::ArcBytes,
	/// Program counter of the `validate_block` export.
	pub entry_pc: u64,
	/// RO data image, poked into the inner PVM's RO region.
	pub ro_data: polkavm::ArcBytes,
	/// Initialised RW data image; the rest of the RW region (BSS + heap arena) is zeroed.
	pub rw_data: polkavm::ArcBytes,
	/// Absolute RO/RW/stack/heap layout the guest was linked against.
	pub memory: polkavm::MemoryMap,
}

// TODO: let the code hash already commit to this instead of the bare code
pub fn parse_pvf(code: &[u8]) -> Option<ParsedPvf> {
	use polkavm::{ArcBytes, MemoryMapBuilder, ProgramBlob, ProgramParts};

	let parts = ProgramParts::from_bytes(ArcBytes::from(code)).ok()?;
	let code = parts.code_and_jump_table.clone();
	let ro_data = parts.ro_data.clone();
	let rw_data = parts.rw_data.clone();

	// `PAGE_SIZE` matches the node's `pages` host call, so these ranges line up with the
	// pages we later allocate.
	let memory = MemoryMapBuilder::new(PAGE_SIZE)
		.ro_data_size(parts.ro_data_size)
		.rw_data_size(parts.rw_data_size)
		.stack_size(parts.stack_size)
		.build()
		.ok()?;

	let program = ProgramBlob::from_parts(parts).ok()?;
	let entry_pc = program.exports().find(|export| export == "validate_block")?.program_counter().0;

	Some(ParsedPvf { code, entry_pc: entry_pc as u64, ro_data, rw_data, memory })
}

/// Instantiate the parsed PVF as an inner PVM, run `validate_block` over the opaque PoV,
/// and return the head data it produces. The PoV is copied in verbatim; the runtime
/// decodes it. `machine` spawns the VM code-only, so we lay out its memory first.
pub fn run(pvf: &ParsedPvf, pov: &[u8]) -> Result<Vec<u8>, RefineLog> {
	let handle =
		refine::machine(&pvf.code[..], pvf.entry_pc).map_err(|_| RefineLog::InvalidCode)?;
	let mem = &pvf.memory;

	// Map + fill the guest's RO, RW (incl. zeroed BSS + heap arena) and stack regions.
	// TODO: map the RO region read-only once poking into protected pages is confirmed.
	alloc_pages(handle, mem.ro_data_address(), mem.ro_data_size())?;
	poke_bytes(handle, mem.ro_data_address(), &pvf.ro_data)?;
	alloc_pages(handle, mem.rw_data_address(), mem.rw_data_size())?;
	poke_bytes(handle, mem.rw_data_address(), &pvf.rw_data)?;
	alloc_pages(handle, mem.stack_address_low(), mem.stack_size())?;

	// Drop the opaque PoV into a page-aligned slot in the unused heap area and hand its
	// `(ptr, len)` to `validate_block(ptr, len)`.
	let input_ptr = align_up(mem.heap_base(), PAGE_SIZE);
	alloc_pages(handle, input_ptr, pov.len() as u32)?;
	poke_bytes(handle, input_ptr, pov)?;

	let mut regs = [0u64; 13];
	regs[Reg::SP as usize] = mem.stack_address_high() as u64;
	regs[Reg::RA as usize] = polkavm::RETURN_TO_HOST;
	regs[Reg::A0 as usize] = input_ptr as u64;
	regs[Reg::A1 as usize] = pov.len() as u64;

	let (outcome, _gas, out_regs) = refine::invoke(handle, refine::gas() as i64, regs)
		.map_err(|_| RefineLog::ValidationFailed)?;
	// We pre-map everything, so anything but a clean halt (panic / trap / OOG / page fault
	// / stray host call) means the candidate did not validate.
	let result = match outcome {
		InvokeOutcome::Halt => Ok(()),
		InvokeOutcome::PageFault(_) => Err(RefineLog::ValidationFailed),
		InvokeOutcome::HostCallFault(0) => Err(RefineLog::TODOMockError),
		InvokeOutcome::HostCallFault(_) => Err(RefineLog::ValidationFailed),
		InvokeOutcome::Panic => Err(RefineLog::ValidationFailed),
		InvokeOutcome::OutOfGas => Err(RefineLog::ValidationFailed),
	};
	if let Err(log) = result {
		return Err(log);
	}

	// `validate_block` returns `(ptr, len)` of the encoded head data in A0/A1.
	let head_data = refine::peek(handle, out_regs[Reg::A0 as usize], out_regs[Reg::A1 as usize])
		.map_err(|_| RefineLog::ValidationFailed)?;

	let _ = refine::expunge(handle);
	Ok(head_data)
}

/// Allocate + zero the pages spanning `[addr, addr + len)`; `addr` must be page-aligned.
fn alloc_pages(handle: u64, addr: u32, len: u32) -> Result<(), RefineLog> {
	if len == 0 {
		return Ok(());
	}
	let page = (addr / PAGE_SIZE) as u64;
	let count = len.div_ceil(PAGE_SIZE) as u64;
	refine::zero(handle, page, count, PageMode::ReadWrite).map_err(|_| RefineLog::ValidationFailed)
}

/// Copy `data` into the inner PVM at `addr` (whose page must already be allocated).
fn poke_bytes(handle: u64, addr: u32, data: &[u8]) -> Result<(), RefineLog> {
	if data.is_empty() {
		return Ok(());
	}
	refine::poke(handle, data, addr as u64).map_err(|_| RefineLog::ValidationFailed)
}

fn align_up(addr: u32, align: u32) -> u32 {
	(addr + (align - 1)) & !(align - 1)
}
