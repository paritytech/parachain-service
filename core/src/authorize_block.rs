//! Running the authorizer on a block: how a parachain runtime checks a block's author (issue #62).
//!
//! The authorizer named by the block's [`BlockAuthorization`] has to be in an auth pool at the
//! block's anchor and has to name this para; its code is read from JAM state as a preimage of its
//! code hash and run in a PVM interpreter at its `authorize_block_ext` entry point.

use crate::{
	blake2_256,
	block_authorization::{
		auth_pools_contain, BlockAuthorization, BlockAuthorizationParams, AUTHORIZE_BLOCK_ENTRY_PC,
		AUTH_POOLS_STATE_KEY,
	},
	service_preimage_state_key,
	types::ParaId,
	StateKey,
};
use alloc::vec::Vec;
use codec::Encode;
use polkavm::{InterruptKind, ProgramCounter, Reg};

/// Gas for one `authorize_block_ext` call: what JAM grants one `is_authorized` call.
pub const AUTHORIZE_BLOCK_GAS: i64 = 50_000_000;

/// The `grow_heap` host call, which the guest allocator grows its heap with.
const GROW_HEAP_HOST_CALL: u32 = 1;
/// The non-GP `log` host call. The authorizer logs through it before it panics.
const LOG_HOST_CALL: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizeBlockError {
	/// The auth pools are missing from JAM state or undecodable.
	AuthPoolsUnavailable,
	/// No auth pool at the anchor holds the authorizer.
	NotInAnyPool,
	/// The authorizer config does not name this para.
	ForeignPara,
	/// The authorizer code is missing from JAM state.
	CodeUnavailable,
	/// The code read from JAM state does not hash to the code hash.
	CodeHashMismatch,
	/// The authorizer did not authorize the block.
	Run(RunError),
}

/// Why running the authorizer produced no output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
	InvalidBlob,
	Instantiation,
	MemoryAccess,
	/// The authorizer panicked, i.e. rejected the block.
	Trap,
	OutOfGas,
	/// A host call other than `log`; the entry point gets none.
	HostCall(u32),
}

/// Check `authorization` for the block of `para_id` whose seal-less header hashes to `block_hash`.
///
/// `read` reads JAM state at the block's anchor; `slot` is the JAM slot the authorizer selects the
/// expected author with. Returns the authorizer's output, its trace for this block.
pub fn authorize_block(
	read: impl Fn(&StateKey) -> Option<Vec<u8>>,
	para_id: ParaId,
	authorization: &BlockAuthorization,
	block_hash: [u8; 32],
	slot: u32,
) -> Result<Vec<u8>, AuthorizeBlockError> {
	let pools = read(&AUTH_POOLS_STATE_KEY).ok_or(AuthorizeBlockError::AuthPoolsUnavailable)?;
	let allowed = auth_pools_contain(&pools, &authorization.authorizer_hash())
		.ok_or(AuthorizeBlockError::AuthPoolsUnavailable)?;
	if !allowed {
		return Err(AuthorizeBlockError::NotInAnyPool);
	}

	// The pools of all cores are searched, so an authorizer of another para is in there as well.
	if !authorization.para_ids().is_some_and(|ids| ids.contains(&para_id)) {
		return Err(AuthorizeBlockError::ForeignPara);
	}

	let code =
		read(&service_preimage_state_key(authorization.auth_code_host, &authorization.code_hash))
			.ok_or(AuthorizeBlockError::CodeUnavailable)?;
	if blake2_256(&code) != authorization.code_hash {
		return Err(AuthorizeBlockError::CodeHashMismatch);
	}

	let params = BlockAuthorizationParams {
		config: authorization.config.clone(),
		token: authorization.token.clone(),
		block_hash,
		slot,
	};
	run(&code, &params.encode()).map_err(AuthorizeBlockError::Run)
}

/// Run the JAM program blob `code` at its `authorize_block_ext` entry point on `input`.
pub fn run(code: &[u8], input: &[u8]) -> Result<Vec<u8>, RunError> {
	let blob =
		jam_program_blob_common::ProgramBlob::from_bytes(code).ok_or(RunError::InvalidBlob)?;
	let blob = polkavm::ProgramBlob::from_parts(blob.into()).map_err(|_| RunError::InvalidBlob)?;

	let mut config = polkavm::Config::new();
	config.set_backend(Some(polkavm::BackendKind::Interpreter));
	let engine = polkavm::Engine::new(&config).map_err(|_| RunError::Instantiation)?;

	let input_len = u32::try_from(input.len()).map_err(|_| RunError::MemoryAccess)?;
	let mut module_config = polkavm::ModuleConfig::default();
	module_config.set_aux_data_size(input_len);
	module_config.set_gas_metering(Some(polkavm::GasMeteringKind::Sync));
	let module = polkavm::Module::from_blob(&engine, &module_config, blob)
		.map_err(|_| RunError::Instantiation)?;

	let mut instance = module.instantiate().map_err(|_| RunError::Instantiation)?;
	let input_address = module.memory_map().aux_data_address();
	instance
		.set_accessible_aux_size(input_len)
		.map_err(|_| RunError::MemoryAccess)?;
	instance
		.write_memory(input_address, input)
		.map_err(|_| RunError::MemoryAccess)?;
	instance.set_gas(AUTHORIZE_BLOCK_GAS);
	instance
		.prepare_call_typed(ProgramCounter(AUTHORIZE_BLOCK_ENTRY_PC), (input_address, input_len));

	loop {
		match instance.run().map_err(|_| RunError::Trap)? {
			InterruptKind::Ecalli(LOG_HOST_CALL) => continue,
			InterruptKind::Ecalli(GROW_HEAP_HOST_CALL) => {
				let requested = instance.reg(Reg::A0);
				let pages = grow_heap(&mut instance, requested)?;
				instance.set_reg(Reg::A0, pages);
			},
			InterruptKind::Ecalli(index) => return Err(RunError::HostCall(index)),
			InterruptKind::Finished => break,
			InterruptKind::NotEnoughGas => return Err(RunError::OutOfGas),
			InterruptKind::Trap | InterruptKind::Segfault(_) | InterruptKind::Step => {
				return Err(RunError::Trap)
			},
		}
	}

	let (Ok(address), Ok(len)) =
		(u32::try_from(instance.reg(Reg::A0)), u32::try_from(instance.reg(Reg::A1)))
	else {
		return Err(RunError::MemoryAccess);
	};
	instance.read_memory(address, len).map_err(|_| RunError::MemoryAccess)
}

/// Grow the heap to `pages` pages, as PolkaJAM's `grow_heap` does: a request below the current size
/// or beyond the address space leaves the heap as it is. Returns the heap size in pages.
fn grow_heap(instance: &mut polkavm::RawInstance, pages: u64) -> Result<u64, RunError> {
	let memory_map = instance.module().memory_map();
	let page_size = u64::from(memory_map.page_size());
	let limit =
		(u64::from(memory_map.heap_base()) + u64::from(memory_map.max_heap_size())) / page_size;

	let top = instance.sbrk(0).ok().flatten().ok_or(RunError::MemoryAccess)?;
	let top_pages = u64::from(top).div_ceil(page_size);
	if pages <= top_pages || pages > limit {
		return Ok(top_pages);
	}

	let delta =
		u32::try_from((pages - top_pages) * page_size).map_err(|_| RunError::MemoryAccess)?;
	let top = instance.sbrk(delta).ok().flatten().ok_or(RunError::MemoryAccess)?;
	Ok(u64::from(top).div_ceil(page_size))
}
