//! Parsing and running a parachain validation function (PVF) as an inner PVM.

use crate::{
	pvf::{executor::ExecutorState, PVF_ENTRY_POINT},
	work_digest::{HeadData, RefineLog},
};
use jam_pvm_common::{refine, InvokeOutcome};
use jam_types::{Hash, PageMode, PAGE_SIZE};
use parachain_service_interface::{types::ParaId, upward_message::UpwardMessages};
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

pub enum PvfParseError {
	InvalidCodeBytes,
	InvalidMemoryMap(&'static str),
	InvalidProgramParts,
	/// Entry point `validate_block` not found in the program.
	MissingEntryPoint,
}

// TODO: let the code hash already commit to this instead of the bare code
pub fn parse_pvf(code: &[u8]) -> Result<ParsedPvf, PvfParseError> {
	use polkavm::{ArcBytes, MemoryMapBuilder, ProgramBlob, ProgramParts};

	// TODO the inner parse error is opaque for some reason?!
	let parts = ProgramParts::from_bytes(ArcBytes::from(code))
		.map_err(|_| PvfParseError::InvalidCodeBytes)?;
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
		.map_err(|e| PvfParseError::InvalidMemoryMap(e))?;

	let program = ProgramBlob::from_parts(parts).map_err(|_| PvfParseError::InvalidProgramParts)?;
	// TODO check if there is a better way than to just search
	let entry_pc = program
		.exports()
		.find(|export| export == PVF_ENTRY_POINT)
		.ok_or(PvfParseError::MissingEntryPoint)?
		.program_counter()
		.0;

	Ok(ParsedPvf { code, entry_pc: entry_pc as u64, ro_data, rw_data, memory })
}

/// Instantiate the parsed PVF as an inner PVM and invoke `jam_validate_block()` (spec §4.2).
/// The entry point takes no arguments; the PVF reads its inputs through the
/// `work_item_payload` host call and declares its results through `set_parent_head_hash`
/// and `set_head` (DECISIONS.md D-1). `machine` spawns the VM code-only, so we lay out
/// its memory first.
pub fn run(
	pvf: &ParsedPvf,
	para_id: ParaId,
) -> Result<(Hash, HeadData, UpwardMessages), RefineLog> {
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

	let mut regs = [0u64; 13];
	regs[Reg::SP as usize] = mem.stack_address_high() as u64;
	regs[Reg::RA as usize] = polkavm::RETURN_TO_HOST;

	let mut exe = ExecutorState::new(para_id);

	let result = loop {
		let (outcome, _gas, out_regs) = match refine::invoke(handle, refine::gas() as i64, regs) {
			Ok(r) => r,
			Err(_) => break Err(RefineLog::ValidationFailed),
		};
		regs = out_regs;

		match outcome {
			InvokeOutcome::Halt => break Ok(()),
			InvokeOutcome::HostCallFault(index) => {
				if let Err(e) = exe.dispatch(handle, index, &mut regs) {
					break Err(e);
				}
			},
			// A PVF that page-faults, panics, or runs out of gas failed to
			// validate the candidate (§4.2).
			InvokeOutcome::PageFault(_) | InvokeOutcome::Panic | InvokeOutcome::OutOfGas => {
				break Err(RefineLog::ValidationFailed)
			},
		}
	};

	let _ = refine::expunge(handle);
	result?;
	exe.finish()
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
