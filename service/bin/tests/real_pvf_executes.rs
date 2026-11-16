//! Proves the repo's own `frameless` runtime blob executes as a child PVF under the
//! service's real executor (`service/src/pvf/pvm.rs::run`), outside any JAM network:
//! parse (polkavm 0.37 vs the blob's linker), inner-PVM instantiation, and
//! `jam_validate_block`'s read/declare host-call surface.
//!
//! The service runs here as native Rust. Its child-PVM host calls (`compile`, `machine`,
//! `invoke`, `peek`, `poke`, `pages`, `gas`, `grow_heap`, `expunge`, `export`, `fetch`,
//! `historical_lookup`, `log`) are JAM imports that only link inside a guest build; the
//! shims below provide them, backed by real polkavm 0.37 modules and `RawInstance`s in
//! thread-local state.
//!
//! `grow_heap` follows PolkaJam's page semantics: the argument is the absolute target heap
//! size in pages and the result is the heap's resulting (or, when refused, current) page
//! count. `log` serves both the service's own diagnostics and the frameless guest's panic
//! handler, which logs through the non-GP `log` host call at index 100.
//!
//! The frameless guest reads its PoV as work-item extrinsic 0 (fetch kind 4,
//! `OurExtrinsic`) and declares its results through the mandatory `set_parent_head_hash` (200)
//! and `set_head` (201) host calls. Unlike the SDK runtime this used to drive, frameless
//! needs no relay-chain state, so `jam_validate_block` completes — `run` returns the new
//! head and the parent-head hash, exactly as `refine.rs` asserts end-to-end.

use std::{cell::RefCell, collections::HashMap};

use codec::{Decode, Encode};
use frameless::{blake2_256, hash_state, BlockData, Config, HeadData, State, ValidationParams};
use jam_types::{InvokeOutcomeCode, SimpleResultCode};
use jam_pvm_common::refine::{self, InstanceHandle, ModuleHandle};
use parachain_service::pvf::{
	executor::{host_call_gas_cost, ExecutorState, Heap},
	pvm::{parse_pvf, run, REFINE_GAS_RESERVE},
};
use parachain_service_core::{host_call::HostCall, types::ParaId, upward_message::UpwardMessages};
use polkavm::{
	program::InstructionSet, ArcBytes, GasMeteringKind, InterruptKind, MemoryProtection,
	ModuleConfig, ProgramBlob, ProgramCounter, ProgramParts, Reg,
};

// --- Thread-local inner-PVM state -------------------------------------------

thread_local! {
	static VM: RefCell<Option<VmState>> = const { RefCell::new(None) };
}

struct VmState {
	engine: polkavm::Engine,
	modules: HashMap<u64, (polkavm::Module, ProgramBlob, bool)>,
	instances: HashMap<u64, polkavm::RawInstance>,
	instance_modules: HashMap<u64, u64>,
	next_handle: u64,
	gas_remaining: i64,
	/// The PoV, served for `fetch` kind 4 (`OurExtrinsic(0)`).
	pov: Vec<u8>,
	preimages: HashMap<(u64, [u8; 32]), Vec<u8>>,
	logs: Vec<String>,
	invocations: Vec<(u64, i64, i64)>,
	/// `(trap PC, exit kind)` captured by `invoke` for the report when the guest panics.
	trap: Option<(u32, &'static str)>,
	/// The inner PVM currently (or most recently) running. `grow_heap` has no handle
	/// argument, so it serves this instance.
	current: Option<u64>,
	/// The current inner PVM's heap size in pages, as `grow_heap` sees it: an absolute
	/// page count (relative to address zero), initialized from the instance's memory map
	/// and advanced by each successful `grow_heap`.
	grown_pages: u64,
}

/// ABI of the `invoke` host call's `InvokeArgs` (`jam-types/src/pvm.rs`, `#[repr(C)]`):
/// `gas: i64` then `regs: [u64; 13]`.
#[repr(C)]
struct InvokeArgs {
	gas: i64,
	regs: [u64; 13],
}

fn with_vm<R>(f: impl FnOnce(&mut VmState) -> R) -> R {
	VM.with(|cell| {
		let mut slot = cell.borrow_mut();
		if slot.is_none() {
			let mut config = polkavm::Config::new();
			config.set_allow_dynamic_paging(true);
			config.set_backend(Some(polkavm::BackendKind::Interpreter));
			*slot = Some(VmState {
				engine: polkavm::Engine::new(&config)
					.expect("interpreter engine initializes on this host"),
				modules: HashMap::new(),
				instance_modules: HashMap::new(),
				instances: HashMap::new(),
				next_handle: 0,
				gas_remaining: 0,
				pov: Vec::new(),
				preimages: HashMap::new(),
				logs: Vec::new(),
				invocations: Vec::new(),
				trap: None,
				current: None,
				grown_pages: 0,
			});
		}
		f(slot.as_mut().expect("initialized just above; qed"))
	})
}

fn inst_mut(handle: u64) -> *mut polkavm::RawInstance {
	VM.with(|cell| {
		let mut slot = cell.borrow_mut();
		let vm = slot.as_mut().expect("VM state is initialized before any invoke; qed");
		vm.instances.get_mut(&handle).expect("inner PVM handle is live; qed") as *mut _
	})
}

// --- Child-PVM host-call shims ----------------------------------------------

#[no_mangle]
pub extern "C" fn log(
	_level: u64,
	target_ptr: *const u8,
	target_len: u64,
	text_ptr: *const u8,
	text_len: u64,
) {
	let target = if target_len == 0 {
		&[][..]
	} else {
		unsafe { std::slice::from_raw_parts(target_ptr, target_len as usize) }
	};
	let msg = if text_len == 0 {
		&[][..]
	} else {
		unsafe { std::slice::from_raw_parts(text_ptr, text_len as usize) }
	};
	with_vm(|vm| {
		vm.logs.push(format!(
			"[{}] {}",
			String::from_utf8_lossy(target),
			String::from_utf8_lossy(msg)
		));
	});
}

#[no_mangle]
pub extern "C" fn gas() -> u64 {
	with_vm(|vm| vm.gas_remaining.max(0) as u64)
}

/// The JAM `grow_heap` host call (index 1) with PolkaJam's page semantics: `pages` is the
/// absolute target heap size in pages and the return value is the heap's resulting page
/// count, or its current count when the request is refused (`pages` is not above the
/// current count, or exceeds the address-space limit). The interpreter backend does not
/// implement `sbrk`, so the pages backing the growth are mapped explicitly.
#[no_mangle]
pub extern "C" fn grow_heap(pages: u64) -> u64 {
	with_vm(|vm| {
		let Some(handle) = vm.current else {
			return 0;
		};
		let current = vm.grown_pages;
		let inst = vm
			.instances
			.get_mut(&handle)
			.expect("the running inner PVM handle is live; qed");
		let map = inst.module().memory_map();
		let page_size = u64::from(map.page_size());
		let heap_base = u64::from(map.heap_base());
		let address_space_limit = (heap_base + u64::from(map.max_heap_size())) / page_size;
		if pages <= current || pages > address_space_limit {
			return current;
		}
		let start = heap_base + current * page_size;
		let length = (pages - current) * page_size;
		inst.zero_memory_with_memory_protection(
			start as u32,
			length as u32,
			MemoryProtection::ReadWrite,
		)
		.unwrap_or_else(|_| panic!("inner-PVM grow_heap page mapping failed"));
		vm.grown_pages = pages;
		pages
	})
}

fn valid_entry(blob: &ProgramBlob, pc: u64) -> bool {
	let code = blob.code();
	if pc >= code.len() as u64 {
		return false;
	}
	let offset = pc as usize;
	blob.bitmask()
		.get(offset / 8)
		.is_some_and(|byte| (byte >> (offset % 8)) & 1 == 1) &&
		blob.isa().opcode_from_raw(code[offset].into()).is_some()
}

#[no_mangle]
pub extern "C" fn compile(mode: u64, code_ptr: *const u8, code_len: u64, offset: u64, service: u64) -> u64 {
	if with_vm(|vm| vm.modules.len() >= 128) {
		return SimpleResultCode::StorageFull as u64;
	}
	let code = match mode {
		// Code bytes from caller memory.
		0 => unsafe { std::slice::from_raw_parts(code_ptr, code_len as usize) }.to_vec(),
		// Historical preimage slice by hash.
		1 => {
			let mut hash = [0; 32];
			unsafe { std::ptr::copy_nonoverlapping(code_ptr, hash.as_mut_ptr(), hash.len()) };
			let Some(preimage) = with_vm(|vm| vm.preimages.get(&(service, hash)).cloned()) else {
				return SimpleResultCode::Nothing as u64;
			};
			let start = offset.min(preimage.len() as u64) as usize;
			let end = offset.saturating_add(code_len).min(preimage.len() as u64) as usize;
			preimage[start..end.max(start)].to_vec()
		},
		_ => return SimpleResultCode::ActionInvalid as u64,
	};
	with_vm(|vm| {
		let mut parts = ProgramParts::empty(polkavm::program::InstructionSetKind::JamV1);
		parts.code_and_jump_table = ArcBytes::from(code.as_slice());
		let Ok(blob) = ProgramBlob::from_parts(parts) else {
			return SimpleResultCode::ActionInvalid as u64;
		};
		if !valid_entry(&blob, 0) {
			return SimpleResultCode::ActionInvalid as u64;
		}
		let mut config = ModuleConfig::new();
		config.set_gas_metering(Some(GasMeteringKind::Sync));
		config.set_dynamic_paging(true);
		let Ok(module) = polkavm::Module::from_blob(&vm.engine, &config, blob.clone()) else {
			return SimpleResultCode::ActionInvalid as u64;
		};
		let handle = vm.next_handle;
		vm.next_handle += 1;
		vm.modules.insert(handle, (module, blob, true));
		handle
	})
}

#[no_mangle]
pub extern "C" fn machine(module_handle: u64, pc: u64) -> u64 {
	with_vm(|vm| {
		if vm.instances.len() >= 63 {
			return SimpleResultCode::StorageFull as u64;
		}
		let Some((module, blob, true)) = vm.modules.get(&module_handle) else {
			return SimpleResultCode::IndexUnknown as u64;
		};
		if !valid_entry(blob, pc) {
			return SimpleResultCode::ActionInvalid as u64;
		}
		let mut instance = module.instantiate().expect("inner PVM instantiates");
		instance.set_next_program_counter(ProgramCounter(pc as u32));
		let handle = vm.next_handle;
		vm.next_handle += 1;
		let map = instance.module().memory_map();
		vm.grown_pages = u64::from(map.heap_base()).div_ceil(u64::from(map.page_size()));
		vm.current = Some(handle);
		vm.instances.insert(handle, instance);
		vm.instance_modules.insert(handle, module_handle);
		handle
	})
}

#[no_mangle]
pub extern "C" fn peek(vm_handle: u64, outer_dst: *mut u8, inner_src: u64, length: u64) -> u64 {
	let Some(inst) = with_vm(|vm| vm.instances.get_mut(&vm_handle).map(|i| i as *mut polkavm::RawInstance)) else {
		return SimpleResultCode::IndexUnknown as u64;
	};
	let (Ok(src), Ok(len)) = (u32::try_from(inner_src), u32::try_from(length)) else {
		return SimpleResultCode::OutOfBounds as u64;
	};
	let Ok(bytes) = (unsafe { &mut *inst }).read_memory(src, len) else {
		return SimpleResultCode::OutOfBounds as u64;
	};
	unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), outer_dst, bytes.len()) };
	SimpleResultCode::Ok as u64
}

#[no_mangle]
pub extern "C" fn poke(vm_handle: u64, outer_src: *const u8, inner_dst: u64, length: u64) -> u64 {
	let Some(inst) = with_vm(|vm| vm.instances.get_mut(&vm_handle).map(|i| i as *mut polkavm::RawInstance)) else {
		return SimpleResultCode::IndexUnknown as u64;
	};
	let (Ok(dst), Ok(len)) = (u32::try_from(inner_dst), usize::try_from(length)) else {
		return SimpleResultCode::OutOfBounds as u64;
	};
	let bytes = unsafe { std::slice::from_raw_parts(outer_src, len) };
	if (unsafe { &mut *inst }).write_memory(dst, bytes).is_err() {
		return SimpleResultCode::OutOfBounds as u64;
	}
	SimpleResultCode::Ok as u64
}

#[no_mangle]
pub extern "C" fn pages(vm_handle: u64, page: u64, count: u64, operation: u64) -> u64 {
	const PAGE: u64 = 4096;
	let Some(inst) = with_vm(|vm| vm.instances.get_mut(&vm_handle).map(|i| i as *mut polkavm::RawInstance)) else {
		return SimpleResultCode::IndexUnknown as u64;
	};
	if page < 16 || page > u32::MAX as u64 / PAGE ||
		count > u32::MAX as u64 / PAGE ||
		page + count > u32::MAX as u64 / PAGE || operation > 4 {
		return SimpleResultCode::ActionInvalid as u64;
	}
	let address = (page * PAGE) as u32;
	let length = (count * PAGE) as u32;
	let inst = unsafe { &mut *inst };
	if (operation == 3 || operation == 4) &&
		!inst.is_memory_accessible(address, length, MemoryProtection::Read) {
		return SimpleResultCode::ActionInvalid as u64;
	}
	match operation {
		// PageOperation::Alloc(mode): ReadOnly=1, ReadWrite=2.
		1 => {
			inst.zero_memory_with_memory_protection(address, length, MemoryProtection::Read)
				.unwrap_or_else(|_| panic!("inner-PVM page alloc (RO) failed"));
		},
		2 => {
			inst.zero_memory_with_memory_protection(address, length, MemoryProtection::ReadWrite)
				.unwrap_or_else(|_| panic!("inner-PVM page alloc (RW) failed"));
		},
		// PageOperation::Free = 0.
		0 => {
			let _ = inst.free_pages(address, length);
		},
		// PageOperation::SetMode: ReadOnly=3, ReadWrite=4.
		3 => inst
			.protect_memory(address, length)
			.unwrap_or_else(|_| panic!("protect failed")),
		4 => inst
			.unprotect_memory(address, length)
			.unwrap_or_else(|_| panic!("unprotect failed")),
		_ => panic!("unknown pages operation {operation}"),
	}
	0
}

#[allow(improper_ctypes_definitions)]
#[no_mangle]
pub extern "C" fn invoke(vm_handle: u64, args: *mut core::ffi::c_void) -> (u64, u64) {
	with_vm(|vm| vm.current = Some(vm_handle));
	let args = unsafe { &mut *(args as *mut InvokeArgs) };
	let lent = args.gas;
	let inst = unsafe { &mut *inst_mut(vm_handle) };
	if inst.next_program_counter().is_none() {
		inst.set_next_program_counter(ProgramCounter(0));
	}
	inst.set_gas(args.gas);
	for (reg, value) in Reg::ALL.into_iter().zip(args.regs.iter().copied()) {
		inst.set_reg(reg, value);
	}
	let kind = match inst.run() {
		Ok(kind) => kind,
		Err(error) => panic!("inner-PVM run failed: {error}"),
	};
	args.gas = inst.gas();
	with_vm(|vm| vm.invocations.push((vm_handle, lent, args.gas)));
	for (reg, value) in Reg::ALL.into_iter().zip(args.regs.iter_mut()) {
		*value = inst.reg(reg);
	}
	match kind {
		InterruptKind::Finished => (InvokeOutcomeCode::Halt as u64, 0),
		InterruptKind::Ecalli(index) => (InvokeOutcomeCode::HostCallFault as u64, index as i32 as i64 as u64),
		InterruptKind::Trap => {
			let pc = inst.program_counter().map(|p| p.0).unwrap_or(0);
			with_vm(|vm| vm.trap = Some((pc, "guest panic/trap")));
			(InvokeOutcomeCode::Panic as u64, 0)
		},
		InterruptKind::Segfault(fault) => {
			let pc = inst.program_counter().map(|p| p.0).unwrap_or(0);
			with_vm(|vm| vm.trap = Some((pc, "page fault")));
			(InvokeOutcomeCode::PageFault as u64, fault.page_address as u64)
		},
		InterruptKind::NotEnoughGas => {
			let pc = inst.program_counter().map(|p| p.0).unwrap_or(0);
			with_vm(|vm| vm.trap = Some((pc, "out of gas")));
			(InvokeOutcomeCode::OutOfGas as u64, 0)
		},
		InterruptKind::Step => unreachable!("step tracing is not enabled"),
	}
}

#[no_mangle]
pub extern "C" fn expunge(mode: u64, handle: u64) -> u64 {
	with_vm(|vm| match mode {
		// Remove an instance.
		0 => match vm.instances.remove(&handle) {
			Some(instance) => {
				let module = vm.instance_modules.remove(&handle).expect("instance has a module");
				if !vm.modules[&module].2 && !vm.instance_modules.values().any(|id| *id == module) {
					vm.modules.remove(&module);
				}
				instance.next_program_counter().map_or(0, |pc| pc.0 as u64)
			},
			None => SimpleResultCode::IndexUnknown as u64,
		},
		// Release a module handle.
		1 => {
			let Some((_, _, held)) = vm.modules.get_mut(&handle) else {
				return SimpleResultCode::IndexUnknown as u64;
			};
			if !*held { return SimpleResultCode::IndexUnknown as u64; }
			*held = false;
			if !vm.instance_modules.values().any(|id| *id == handle) {
				vm.modules.remove(&handle);
			}
			0
		},
		// Invalid expunge mode.
		_ => SimpleResultCode::ActionInvalid as u64,
	})
}

#[no_mangle]
pub extern "C" fn export(buffer: *const u8, buffer_len: u64) -> u64 {
	let _ = (buffer, buffer_len);
	0
}

#[no_mangle]
pub extern "C" fn fetch(
	buffer: *mut u8,
	offset: u64,
	buffer_len: u64,
	kind: u64,
	index: u64,
	_b: u64,
) -> u64 {
	// Only the PoV (kind 4 `OurExtrinsic`, extrinsic 0) is served; anything else is
	// absent (`u64::MAX`).
	if kind != 4 || index != 0 {
		return u64::MAX;
	}
	with_vm(|vm| {
		let pov = &vm.pov;
		let full = pov.len() as u64;
		if offset >= full {
			return full;
		}
		let n = (full - offset).min(buffer_len) as usize;
		unsafe { std::ptr::copy_nonoverlapping(pov.as_ptr().add(offset as usize), buffer, n) };
		full
	})
}

#[no_mangle]
pub extern "C" fn historical_lookup(
	service: u64,
	hash_ptr: *const u8,
	out: *mut u8,
	offset: u64,
	out_len: u64,
) -> u64 {
	let mut hash = [0u8; 32];
	unsafe { std::ptr::copy_nonoverlapping(hash_ptr, hash.as_mut_ptr(), 32) };
	with_vm(|vm| match vm.preimages.get(&(service, hash)) {
		Some(preimage) => {
			let full = preimage.len() as u64;
			if offset >= full {
				return full;
			}
			let n = (full - offset).min(out_len) as usize;
			unsafe {
				std::ptr::copy_nonoverlapping(preimage.as_ptr().add(offset as usize), out, n)
			};
			full
		},
		None => u64::MAX,
	})
}

// --- The test ----------------------------------------------------------------

#[test]
fn real_runtime_executes_as_child_pvf() {
	// The repo's own freshly built frameless blob, not a foreign SDK runtime.
	let pvf = parachain_service_bin::frameless_pvf();
	let parsed = match parse_pvf(&pvf) {
		Ok(parsed) => parsed,
		Err(_) => panic!("polkavm 0.37 failed to parse the frameless blob"),
	};

	// One Coretime block (`counter += 512`) on top of genesis — the same shape
	// `refine.rs::run_block` drives end-to-end, only with the PoV hand-served.
	let config = Config::Coretime;
	let parent = HeadData {
		number: 0,
		parent_hash: [0; 32],
		post_state: hash_state(&State { config: config.clone(), counter: 0 }),
	};
	let block = BlockData { state: State { config, counter: 0 }, add: 512 };
	let params = ValidationParams { parent_head: parent.encode(), block_data: block.encode() };

	with_vm(|vm| {
		// The PoV `jam_validate_block` reads as work-item extrinsic 0 (§3.2).
		vm.pov = params.encode();
		vm.gas_remaining = 1_000_000_000;
	});

	let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		run(&parsed, ParaId::new(0), 1_000_000_000)
	}));

	with_vm(|vm| {
		assert!(vm.modules.is_empty(), "compiled module must be expunged");
		assert!(vm.instances.is_empty(), "inner instance must be expunged");
		// The guest reads its PoV via fetch kind 4 (`OurExtrinsic(0)`) and declares its
		// results through the mandatory `set_parent_head_hash` (200) + `set_head` (201) —
		// observed through the executor's own probe log lines.
		let fetched = vm.logs.iter().any(|line| line.contains("fetch probe: kind=4 "));
		let declared = vm.logs.iter().any(|line| line.contains("dispatch probe: call=200"));
		let set_head = vm.logs.iter().any(|line| line.contains("dispatch probe: call=201"));
		println!("host-call log:\n{}", vm.logs.join("\n"));
		println!("PoV extrinsic fetched (kind 4): {fetched}");
		println!("set_parent_head_hash reached: {declared}");
		println!("set_head reached: {set_head}");
		println!("abnormal exit: {:?}", vm.trap);
		assert!(fetched, "the runtime must fetch its PoV as work-item extrinsic 0 via kind 4");
		assert!(declared, "the runtime must decode the candidate and declare the parent head");
		assert!(set_head, "the runtime must set the new head after the parent declaration");
	});

	// Frameless needs no relay-chain state, so `jam_validate_block` completes: run returns the
	// new head and the parent-head hash (blake2-256 over the encoded parent head, D-5).
	let (parent_head_hash, head_data, upward_messages) = match outcome {
		Ok(Ok(ok)) => ok,
		Ok(Err(err)) => panic!("jam_validate_block returned RefineLog::{err:?}"),
		Err(panic) => {
			let msg = panic
				.downcast_ref::<String>()
				.cloned()
				.or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
				.unwrap_or_else(|| "<non-string panic>".to_owned());
			with_vm(|vm| {
				panic!(
					"the runtime panicked instead of completing:\n  panic: {msg}\n  trap: {:?}",
					vm.trap
				)
			})
		},
	};
	assert_eq!(parent_head_hash, blake2_256(&params.parent_head));
	assert_eq!(upward_messages, UpwardMessages::new());
	let head =
		HeadData::decode(&mut &head_data.into_inner()[..]).expect("run returned valid HeadData");
	assert_eq!(head.number, 1);
	assert_eq!(head.parent_hash, parent.hash());
	assert_eq!(head.post_state, hash_state(&State { config: Config::Coretime, counter: 512 }));
}

// The outer caller and every nested machine below are actual interpreter instances.
// Their handle spaces intentionally differ: only the dispatcher sees PVF handles.
const TEST_ADDR: u64 = 0x10_0000;
const CHILD_ADDR: u64 = 0x20_0000;
const A0: usize = 7;
const A1: usize = 8;

fn code(asm: &str) -> Vec<u8> {
	let blob = polkavm::program::assemble(Some(polkavm::program::InstructionSetKind::JamV1), asm)
		.expect("test assembly");
	ProgramParts::from_bytes(blob.into()).expect("assembled blob").code_and_jump_table.to_vec()
}

struct NestedHarness {
	executor: ExecutorState,
	caller: InstanceHandle,
	_module: ModuleHandle,
	gas: i64,
}

impl NestedHarness {
	fn new() -> Self {
		let outer_code = code("trap");
		let module = refine::compile_from_bytes(&outer_code).expect("outer module");
		let caller = refine::machine(&module, 0).expect("outer machine");
		refine::zero(&caller, TEST_ADDR / 4096, 32, jam_types::PageMode::ReadWrite)
			.expect("outer writable memory");
		let heap = with_vm(|vm| Heap::new(vm.instances.get(&caller.id()).unwrap().module().memory_map()));
		Self { executor: ExecutorState::new(ParaId::new(0), heap), caller, _module: module, gas: 100_000_000 }
	}

	fn write(&self, address: u64, bytes: &[u8]) {
		refine::poke(&self.caller, bytes, address).expect("write caller memory");
	}

	fn read(&self, address: u64, len: usize) -> Vec<u8> {
		refine::peek(&self.caller, address, len as u64).expect("read caller memory")
	}

	fn call(&mut self, call: HostCall, args: &[u64]) -> [u64; 13] {
		let mut regs = [0; 13];
		regs[A0..A0 + args.len()].copy_from_slice(args);
		self.executor.dispatch(&self.caller, call as u64, &mut self.gas, &mut regs).expect("known nested host call");
		regs
	}

	fn compile(&mut self, bytes: &[u8]) -> u64 {
		self.write(TEST_ADDR, bytes);
		self.call(HostCall::Compile, &[0, TEST_ADDR, bytes.len() as u64, 0, 0])[A0]
	}

	fn machine(&mut self, module: u64) -> u64 {
		self.call(HostCall::Machine, &[module, 0])[A0]
	}

	fn args(&self, gas: i64, ra: u64) {
		let mut args = [0u8; 112];
		args[..8].copy_from_slice(&gas.to_le_bytes());
		args[8..16].copy_from_slice(&ra.to_le_bytes());
		self.write(TEST_ADDR + 0x10000, &args);
	}

	fn invoke(&mut self, handle: u64) -> [u64; 13] {
		self.call(HostCall::Invoke, &[handle, TEST_ADDR + 0x10000])
	}
}

#[test]
fn nested_compile_and_lifecycle_works() {
	let mut h = NestedHarness::new();
	let bytes = code("ret");
	let module = h.compile(&bytes);
	assert_eq!(module, 0);
	assert_eq!(h.call(HostCall::Machine, &[module, u64::MAX])[A0], SimpleResultCode::ActionInvalid as u64);
	assert_eq!(h.call(HostCall::Expunge, &[9, module])[A0], SimpleResultCode::ActionInvalid as u64);
	let vm = h.machine(module);
	assert_eq!(vm, 0);
	assert_eq!(h.call(HostCall::Expunge, &[1, module])[A0], 0);
	assert_eq!(h.call(HostCall::Machine, &[module, 0])[A0], SimpleResultCode::IndexUnknown as u64);
	// An instance retains its code after its module handle has been removed.
	h.args(100_000, polkavm::RETURN_TO_HOST);
	assert_eq!(h.invoke(vm)[A0], InvokeOutcomeCode::Halt as u64);
	assert_eq!(h.call(HostCall::Expunge, &[0, vm])[A0], 0);
	assert_eq!(h.call(HostCall::Expunge, &[0, vm])[A0], SimpleResultCode::IndexUnknown as u64);
	assert_eq!(h.compile(&bytes), 0);
}

#[test]
fn nested_compile_preimages_works() {
	let mut h = NestedHarness::new();
	let bytes = code("ret");
	let hash = [0x39; 32];
	let mut preimage = vec![5, 6, 7];
	preimage.extend_from_slice(&bytes);
	preimage.extend_from_slice(&[8, 9]);
	with_vm(|vm| {
		vm.preimages.insert((u64::MAX, hash), preimage.clone());
		vm.preimages.insert((42, hash), preimage);
	});
	h.write(TEST_ADDR, &hash);
	for service in [u64::MAX, 42] {
		let module = h.call(HostCall::Compile, &[1, TEST_ADDR, bytes.len() as u64, 3, service])[A0];
		assert!(module < 127);
		assert_eq!(h.call(HostCall::Expunge, &[1, module])[A0], 0);
	}
	assert_eq!(h.call(HostCall::Compile, &[1, TEST_ADDR, bytes.len() as u64, 3, 43])[A0], SimpleResultCode::Nothing as u64);
	h.write(TEST_ADDR, &[0; 32]);
	assert_eq!(h.call(HostCall::Compile, &[1, TEST_ADDR, bytes.len() as u64, 3, 42])[A0], SimpleResultCode::Nothing as u64);
	h.write(TEST_ADDR, &[0xff]);
	assert_eq!(h.call(HostCall::Compile, &[0, TEST_ADDR, 1, 0, 0])[A0], SimpleResultCode::ActionInvalid as u64);
	assert_eq!(h.call(HostCall::Compile, &[2, TEST_ADDR, 0, 0, 0])[A0], SimpleResultCode::ActionInvalid as u64);
}

#[test]
fn nested_memory_and_pages_works() {
	let mut h = NestedHarness::new();
	let module = h.compile(&code("ret"));
	let vm = h.machine(module);
	let page = CHILD_ADDR / 4096;
	let data = b"nested copy";
	h.write(TEST_ADDR + 0x1000, data);
	{
		assert_eq!(h.call(HostCall::Pages, &[vm, page, 1, 2])[A0], 0);
		assert_eq!(h.call(HostCall::Poke, &[vm, TEST_ADDR + 0x1000, CHILD_ADDR, data.len() as u64])[A0], 0);
		assert_eq!(h.call(HostCall::Peek, &[vm, TEST_ADDR + 0x2000, CHILD_ADDR, data.len() as u64])[A0], 0);
		assert_eq!(h.read(TEST_ADDR + 0x2000, data.len()), data);
		assert_eq!(h.call(HostCall::Poke, &[vm, TEST_ADDR + 0x1000, CHILD_ADDR + 4095, 2])[A0], SimpleResultCode::OutOfBounds as u64);
		assert_eq!(h.call(HostCall::Peek, &[vm, TEST_ADDR + 0x2000, CHILD_ADDR + 4095, 2])[A0], SimpleResultCode::OutOfBounds as u64);
	}
	{
		assert_eq!(h.call(HostCall::Pages, &[vm, page, 1, 3])[A0], 0);
		assert_eq!(h.call(HostCall::Poke, &[vm, TEST_ADDR + 0x1000, CHILD_ADDR, 1])[A0], SimpleResultCode::OutOfBounds as u64);
		assert_eq!(h.call(HostCall::Peek, &[vm, TEST_ADDR + 0x2000, CHILD_ADDR, 1])[A0], 0);
		assert_eq!(h.call(HostCall::Pages, &[vm, page, 1, 4])[A0], 0);
		assert_eq!(h.call(HostCall::Poke, &[vm, TEST_ADDR + 0x1000, CHILD_ADDR, 1])[A0], 0);
		assert_eq!(h.call(HostCall::Pages, &[vm, page, 1, 0])[A0], 0);
		assert_eq!(h.call(HostCall::Peek, &[vm, TEST_ADDR + 0x2000, CHILD_ADDR, 1])[A0], SimpleResultCode::OutOfBounds as u64);
		assert_eq!(h.call(HostCall::Pages, &[vm, page, 1, 4])[A0], SimpleResultCode::ActionInvalid as u64);
		assert_eq!(h.call(HostCall::Pages, &[vm, page, 1, 1])[A0], 0);
		assert_eq!(h.call(HostCall::Poke, &[vm, TEST_ADDR + 0x1000, CHILD_ADDR, 1])[A0], SimpleResultCode::OutOfBounds as u64);
	}
	for args in [[vm, 15, 1, 2], [vm, page, 1, 9], [vm, u64::MAX, 1, 2]] {
		assert_eq!(h.call(HostCall::Pages, &args)[A0], SimpleResultCode::ActionInvalid as u64);
	}
}

#[test]
fn nested_invalid_ranges_and_forged_handles_errors() {
	let mut h = NestedHarness::new();
	let outer = h.caller.id();
	let module = h.compile(&code("ret"));
	let vm = h.machine(module);
	assert_ne!(outer, vm);
	let who = SimpleResultCode::IndexUnknown as u64;
	assert_eq!(h.call(HostCall::Machine, &[outer, 0])[A0], who);
	assert_eq!(h.call(HostCall::Pages, &[outer, TEST_ADDR / 4096, 1, 0])[A0], who);
	assert_eq!(h.call(HostCall::Poke, &[outer, TEST_ADDR, TEST_ADDR, 1])[A0], who);
	assert_eq!(h.call(HostCall::Peek, &[outer, TEST_ADDR + 0x1000, TEST_ADDR, 1])[A0], who);
	assert_eq!(h.call(HostCall::Expunge, &[0, outer])[A0], who);
	assert_eq!(h.call(HostCall::Expunge, &[1, outer])[A0], who);
	h.write(TEST_ADDR + 0x1000, b"still alive");
	assert_eq!(h.read(TEST_ADDR + 0x1000, 11), b"still alive");
	h.args(10, polkavm::RETURN_TO_HOST);
	let args_before = h.read(TEST_ADDR + 0x10000, 112);
	let before = h.gas;
	let result = h.invoke(outer);
	assert_eq!(result[A0], who);
	assert_eq!(result[A1], TEST_ADDR + 0x10000);
	assert_eq!(before - h.gas, host_call_gas_cost(&HostCall::Invoke) + 978);
	assert_eq!(h.read(TEST_ADDR + 0x10000, 112), args_before);
	for call in [HostCall::Peek, HostCall::Poke] {
		let trapped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			h.call(call, &[u64::MAX, 0, CHILD_ADDR, 1]);
		}));
		assert!(trapped.is_err(), "invalid caller memory must trap before handle lookup");
	}
	// A readable but read-only caller destination is not a valid peek destination.
	refine::protect(&h.caller, (TEST_ADDR + 0x2000) / 4096, 1, jam_types::PageMode::ReadOnly)
		.expect("protect caller page");
	let trapped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		h.call(HostCall::Peek, &[u64::MAX, TEST_ADDR + 0x2000, CHILD_ADDR, 1]);
	}));
	assert!(trapped.is_err());
}

#[test]
fn nested_handle_limits_and_retention_works() {
	let mut h = NestedHarness::new();
	let bytes = code("ret");
	let first = h.compile(&bytes);
	let vm = h.machine(first);
	assert_eq!(h.call(HostCall::Expunge, &[1, first])[A0], 0);
	for expected in 1..127 {
		assert_eq!(h.compile(&bytes), expected);
	}
	assert_eq!(h.call(HostCall::Compile, &[0, 0, 1, 0, 0])[A0], SimpleResultCode::StorageFull as u64);
	assert_eq!(h.call(HostCall::Expunge, &[0, vm])[A0], 0);
	assert_eq!(h.compile(&bytes), 0);
	for expected in 0..62 {
		assert_eq!(h.machine(0), expected);
	}
	assert_eq!(h.call(HostCall::Machine, &[u64::MAX, 0])[A0], SimpleResultCode::StorageFull as u64);
	assert_eq!(h.call(HostCall::Expunge, &[0, 12])[A0], 0);
	assert_eq!(h.machine(0), 12);
}

#[test]
fn nested_invoke_gas_and_outcomes_works() {
	let mut h = NestedHarness::new();
	let module = h.compile(&code("ret"));
	let vm = h.machine(module);
	h.args(100_000, polkavm::RETURN_TO_HOST);
	let before = h.gas;
	assert_eq!(h.invoke(vm)[A0], InvokeOutcomeCode::Halt as u64);
	let returned = i64::from_le_bytes(h.read(TEST_ADDR + 0x10000, 8).try_into().unwrap());
	assert!((0..100_000).contains(&returned));
	assert_eq!(before - h.gas, host_call_gas_cost(&HostCall::Invoke) + 968 + 100_000 - returned);
	h.args(100_000, polkavm::RETURN_TO_HOST);
	let before = h.gas;
	assert_eq!(h.invoke(u64::MAX)[A0], SimpleResultCode::IndexUnknown as u64);
	assert_eq!(before - h.gas, host_call_gas_cost(&HostCall::Invoke) + 968 + 100_000);
	h.args(100_000, polkavm::RETURN_TO_HOST);
	h.gas = host_call_gas_cost(&HostCall::Invoke) + 968 + 100_000 - 1;
	let trapped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.invoke(vm)));
	assert!(trapped.is_err());
	assert_eq!(h.gas, 0);
}

#[test]
fn nested_invoke_interrupts_works() {
	for (asm, expected, detail) in [
		("trap", InvokeOutcomeCode::Panic as u64, None),
		("ecalli 42\nret", InvokeOutcomeCode::HostCallFault as u64, Some(42)),
		("a0 = u8 [0x200000]\nret", InvokeOutcomeCode::PageFault as u64, Some(CHILD_ADDR)),
	] {
		let mut h = NestedHarness::new();
		let module = h.compile(&code(asm));
		let vm = h.machine(module);
		h.args(100_000, polkavm::RETURN_TO_HOST);
		let result = h.invoke(vm);
		assert_eq!(result[A0], expected);
		if let Some(detail) = detail { assert_eq!(result[A1], detail); }
	}
	let mut h = NestedHarness::new();
	let module = h.compile(&code("ret"));
	let vm = h.machine(module);
	h.args(0, polkavm::RETURN_TO_HOST);
	assert_eq!(h.invoke(vm)[A0], InvokeOutcomeCode::OutOfGas as u64);
}

#[test]
fn nested_invoke_through_pvf_run_preserves_budget_works() {
	use parachain_service::pvf::pvm::ParsedPvf;

	let child = code("ret");
	let memory = polkavm::MemoryMapBuilder::new(4096)
		.rw_data_size(4096)
		.stack_size(4096)
		.build()
		.expect("PVF memory layout");
	let rw = memory.rw_data_address() as u64;
	let args_address = rw + 256;
	let mut data = vec![0u8; 368];
	data[..child.len()].copy_from_slice(&child);
	data[256..264].copy_from_slice(&100_000i64.to_le_bytes());
	data[264..272].copy_from_slice(&polkavm::RETURN_TO_HOST.to_le_bytes());
	let program = code(&format!(
		"a0 = 0\na1 = {rw}\na2 = {}\na3 = 0\na4 = 0\necalli 9\n\
		 a1 = 0\necalli 10\na1 = {args_address}\necalli 14\nret",
		child.len(),
	));
	let pvf = ParsedPvf {
		code: ArcBytes::from(program.as_slice()),
		entry_pc: 0,
		ro_data: ArcBytes::from(&[][..]),
		rw_data: ArcBytes::from(data.as_slice()),
		memory,
	};
	for outer_gas in [REFINE_GAS_RESERVE + 1_000_000, 800_000] {
		with_vm(|vm| {
			vm.gas_remaining = outer_gas as i64;
			vm.invocations.clear();
		});
		// Deliberately omits mandatory head declarations.
		assert!(run(&pvf, ParaId::new(0), REFINE_GAS_RESERVE + 1_000_000).is_err());
		with_vm(|vm| {
			let outer = vm.invocations[0].0;
			let outer_runs: Vec<_> =
				vm.invocations.iter().filter(|entry| entry.0 == outer).collect();
			assert_eq!(outer_runs.len(), 4, "compile, machine, invoke, then halt");
			assert_eq!(vm.invocations.len(), 5, "nested invoke must actually run its child");
			let nested = vm.invocations.iter().find(|entry| entry.0 != outer).unwrap();
			let mut budget = 1_000_000;
			for (index, (_, lent, remaining)) in outer_runs.iter().enumerate() {
				assert_eq!(*lent, budget.min(outer_gas as i64 - 2048));
				budget -= lent - remaining;
				if let Some(call) = [HostCall::Compile, HostCall::Machine, HostCall::Invoke].get(index) {
					budget -= host_call_gas_cost(call);
				}
				if index == 2 {
					budget -= 968 + nested.1 - nested.2;
				}
			}
		});
	}
}
