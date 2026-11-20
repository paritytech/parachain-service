//! Parsing and running a parachain validation function (PVF) as an inner PVM.
//!
//! Abnormal PVF exits (spawn/invoke failures, page faults, panics, out of gas)
//! are deliberately not caught: they panic the whole refine invocation (§4.2),
//! so no `ParachainWorkDigest` is produced. The returned `Err(RefineLog)` only
//! carries structured spec errors from the child host calls.

use crate::{
	pvf::{
		executor::{ExecutorState, Heap},
		PVF_ENTRY_POINT,
	},
	work_digest::{HeadData, RefineLog},
};
use jam_pvm_common::{refine, InvokeOutcome};
use jam_types::{Hash, PageMode, UnsignedGas, PAGE_SIZE};
use parachain_service_core::{types::ParaId, upward_message::UpwardMessages};
use polkavm::Reg;

/// A parachain validation function parsed into a form ready to run as an inner PVM.
///
/// `machine` spawns the inner PVM code-only, so we also carry its memory image and layout
/// to set up before invoking.
pub struct ParsedPvf {
	/// `code + jump table` blob, as the `compile` host call expects it.
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

/// Covers the 15 MiB PVF probe with 25% headroom, rounded up.
/// Larger memory images may still exhaust Refine's outer gas.
pub const REFINE_GAS_RESERVE: UnsignedGas = 135_000_000;

/// Invoke base cost and handoff slack.
const INVOKE_GAS_RESERVE: UnsignedGas = 2048;

/// Instantiate the parsed PVF as an inner PVM and invoke `jam_validate_block()` (spec §4.2).
/// The entry point takes no arguments; the PVF reads its inputs through the forwarded
/// `fetch` host call (its PoV is work-item extrinsic 0) and declares its results through
/// `set_parent_head_hash` and `set_head` (DECISIONS.md D-1). `machine` spawns the VM code-only, so
/// we lay out its memory first.
///
/// Machine failures (spawn, invoke, page faults, panic, out of gas) panic the whole
/// refine invocation (§4.2); the returned `Err` carries only structured spec errors.
pub fn run(
	pvf: &ParsedPvf,
	para_id: ParaId,
	refine_gas_limit: UnsignedGas,
) -> Result<(Hash, HeadData, UpwardMessages), RefineLog> {
	let mut gas_left = refine_gas_limit
		.checked_sub(REFINE_GAS_RESERVE)
		.and_then(|gas| i64::try_from(gas).ok())
		.expect("PVF gas budget must fit after reserving Refine gas");
	let module = refine::compile_from_bytes(&pvf.code[..])
		.unwrap_or_else(|_| panic!("PVF inner PVM compilation failed; §4.2 whole-refine failure"));
	let handle = refine::machine(&module, pvf.entry_pc).unwrap_or_else(|_| {
		panic!("PVF inner PVM machine spawn failed; §4.2 whole-refine failure")
	});
	drop(module);
	let mem = &pvf.memory;

	// Map + fill the guest's RO, RW (incl. zeroed BSS) and stack regions. The heap
	// beyond the RW data is mapped on demand by the `grow_heap` handler.
	// TODO: map the RO region read-only once poking into protected pages is confirmed.
	alloc_pages(&handle, mem.ro_data_address(), mem.ro_data_size());
	poke_bytes(&handle, mem.ro_data_address(), &pvf.ro_data);
	alloc_pages(&handle, mem.rw_data_address(), mem.rw_data_size());
	poke_bytes(&handle, mem.rw_data_address(), &pvf.rw_data);
	alloc_pages(&handle, mem.stack_address_low(), mem.stack_size());

	let mut regs = [0u64; 13];
	regs[Reg::SP as usize] = mem.stack_address_high() as u64;
	regs[Reg::RA as usize] = polkavm::RETURN_TO_HOST;

	let mut exe = ExecutorState::new(para_id, Heap::new(&pvf.memory));
	let result: Result<(), RefineLog> = loop {
		// Cap the loan, not validation's budget.
		let available = refine::gas()
			.saturating_sub(INVOKE_GAS_RESERVE)
			.min(i64::MAX as u64) as i64;
		let lent = gas_left.min(available);
		let (outcome, remaining, out_regs) = refine::invoke(&handle, lent, regs)
			.unwrap_or_else(|_| panic!("PVF inner PVM invoke failed; §4.2 whole-refine failure"));
		regs = out_regs;
		gas_left = gas_left.saturating_sub(lent.saturating_sub(remaining));
		let tag = match outcome {
			InvokeOutcome::Halt => "halt",
			InvokeOutcome::HostCallFault(_) => "host",
			InvokeOutcome::PageFault(_) => "fault",
			InvokeOutcome::Panic => "panic",
			InvokeOutcome::OutOfGas => "oog",
		};
		jam_pvm_common::info!("PVF invoke probe: {tag}");

		match outcome {
			InvokeOutcome::Halt => break Ok(()),
			InvokeOutcome::HostCallFault(index) => {
				if let Err(e) = exe.dispatch(&handle, index, &mut gas_left, &mut regs) {
					break Err(e);
				}
			},
			// An abnormal PVF exit is deliberately not caught (§4.2): it fails
			// the whole refine invocation instead of producing a digest.
			InvokeOutcome::PageFault(_) | InvokeOutcome::Panic | InvokeOutcome::OutOfGas => {
				panic!("PVF abnormal exit (page fault / panic / out of gas); §4.2 whole-refine failure")
			},
		}
	};

	let _ = refine::expunge(handle);
	match &result {
		Ok(()) => jam_pvm_common::info!("PVF run probe: OK"),
		Err(_) => jam_pvm_common::info!("PVF run probe: Err RefineLog"),
	}
	result?;
	exe.finish()
}

/// Allocate + zero the pages spanning `[addr, addr + len)`; `addr` must be page-aligned.
fn alloc_pages(handle: &refine::InstanceHandle, addr: u32, len: u32) {
	if len == 0 {
		return;
	}
	let page = (addr / PAGE_SIZE) as u64;
	let count = len.div_ceil(PAGE_SIZE) as u64;
	refine::zero(handle, page, count, PageMode::ReadWrite)
		.unwrap_or_else(|_| panic!("PVF page allocation failed; §4.2 whole-refine failure"));
}

/// Copy `data` into the inner PVM at `addr` (whose page must already be allocated).
fn poke_bytes(handle: &refine::InstanceHandle, addr: u32, data: &[u8]) {
	if data.is_empty() {
		return;
	}
	refine::poke(handle, data, addr as u64)
		.unwrap_or_else(|_| panic!("PVF memory poke failed; §4.2 whole-refine failure"));
}
