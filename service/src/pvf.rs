//! Parsing a parachain validation function (PVF) blob into a runnable inner PVM.

use jam_types::PAGE_SIZE;

/// A parachain validation function parsed into a form ready to run as an inner PVM.
///
/// `machine` spawns the inner PVM code-only, so we also carry its memory image and layout
/// to set up before invoking.
#[allow(dead_code)] // TODO: fields consumed once we set up memory and invoke.
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
