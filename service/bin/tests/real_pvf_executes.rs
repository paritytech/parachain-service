//! Proves the repo's own `frameless` runtime blob executes as a child PVF under the
//! service's real executor (`service/src/pvf/pvm.rs::run`), outside any JAM network:
//! parse (polkavm 0.36 vs the blob's linker), inner-PVM instantiation, and
//! `jam_validate_block`'s read/declare host-call surface.
//!
//! The service runs here as native Rust. Its child-PVM host calls (`machine`, `invoke`,
//! `peek`, `poke`, `pages`, `gas`, `expunge`, `export`, `fetch`, `historical_lookup`,
//! `log`) are JAM imports that only link inside a guest build; the shims below provide
//! them, backed by a real polkavm 0.36 `RawInstance` in thread-local state, mirroring
//! what polkajam's own `machine`/`invoke`/`pages` host calls do.
//!
//! The frameless guest reads its PoV as work-item extrinsic 0 (fetch kind 4,
//! `OurExtrinsic`) and declares its results through the mandatory `set_parent_head_hash` (200)
//! and `set_head` (201) host calls. Unlike the SDK runtime this used to drive, frameless
//! needs no relay-chain state, so `jam_validate_block` completes — `run` returns the new
//! head and the parent-head hash, exactly as `refine.rs` asserts end-to-end.

use std::{cell::RefCell, collections::HashMap};

use codec::{Decode, Encode};
use frameless::{blake2_256, hash_state, BlockData, Config, HeadData, State, ValidationParams};
use jam_types::InvokeOutcomeCode;
use parachain_service::pvf::pvm::{parse_pvf, run};
use parachain_service_core::{types::ParaId, upward_message::UpwardMessages};
use polkavm::{
	ArcBytes, GasMeteringKind, InterruptKind, MemoryProtection, ModuleConfig, ProgramBlob,
	ProgramCounter, ProgramParts, Reg,
};

// --- Thread-local inner-PVM state -------------------------------------------

thread_local! {
	static VM: RefCell<Option<VmState>> = const { RefCell::new(None) };
}

struct VmState {
	engine: polkavm::Engine,
	instances: HashMap<u64, polkavm::RawInstance>,
	next_handle: u64,
	gas_remaining: i64,
	/// The PoV, served for `fetch` kind 4 (`OurExtrinsic(0)`).
	pov: Vec<u8>,
	preimages: HashMap<[u8; 32], Vec<u8>>,
	logs: Vec<String>,
	/// `(trap PC, exit kind)` captured by `invoke` for the report when the guest panics.
	trap: Option<(u32, &'static str)>,
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
				instances: HashMap::new(),
				next_handle: 0,
				gas_remaining: 0,
				pov: Vec::new(),
				preimages: HashMap::new(),
				logs: Vec::new(),
				trap: None,
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
	msg_ptr: *const u8,
	msg_len: u64,
) {
	let target = if target_len == 0 {
		&[][..]
	} else {
		unsafe { std::slice::from_raw_parts(target_ptr, target_len as usize) }
	};
	let msg = if msg_len == 0 {
		&[][..]
	} else {
		unsafe { std::slice::from_raw_parts(msg_ptr, msg_len as usize) }
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

#[no_mangle]
pub extern "C" fn machine(code_ptr: *const u8, code_len: u64, pc: u64) -> u64 {
	let code = unsafe { std::slice::from_raw_parts(code_ptr, code_len as usize) };
	with_vm(|vm| {
		let mut parts = ProgramParts::empty(polkavm::program::InstructionSetKind::JamV1);
		parts.code_and_jump_table = ArcBytes::from(code);
		let blob = ProgramBlob::from_parts(parts)
			.expect("the code+jump-table blob reparses as a JamV1 program");
		let mut config = ModuleConfig::new();
		config.set_gas_metering(Some(GasMeteringKind::Sync));
		config.set_dynamic_paging(true);
		let module = polkavm::Module::from_blob(&vm.engine, &config, blob)
			.expect("inner PVM module compiles under the 0.36 interpreter");
		let mut instance = module.instantiate().expect("inner PVM instantiates");
		instance.set_next_program_counter(ProgramCounter(pc as u32));
		let handle = vm.next_handle;
		vm.next_handle += 1;
		vm.instances.insert(handle, instance);
		handle
	})
}

#[no_mangle]
pub extern "C" fn peek(vm_handle: u64, outer_dst: *mut u8, inner_src: u64, length: u64) -> u64 {
	let inst = unsafe { &mut *inst_mut(vm_handle) };
	let bytes = inst
		.read_memory(inner_src as u32, length as u32)
		.unwrap_or_else(|_| panic!("inner-PVM peek failed"));
	unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), outer_dst, bytes.len()) };
	0
}

#[no_mangle]
pub extern "C" fn poke(vm_handle: u64, outer_src: *const u8, inner_dst: u64, length: u64) -> u64 {
	let bytes = unsafe { std::slice::from_raw_parts(outer_src, length as usize) };
	let inst = unsafe { &mut *inst_mut(vm_handle) };
	inst.write_memory(inner_dst as u32, bytes)
		.unwrap_or_else(|_| panic!("inner-PVM poke failed"));
	0
}

#[no_mangle]
pub extern "C" fn pages(vm_handle: u64, page: u64, count: u64, operation: u64) -> u64 {
	const PAGE: u64 = 4096;
	let address = (page * PAGE) as u32;
	let length = (count * PAGE) as u32;
	let inst = unsafe { &mut *inst_mut(vm_handle) };
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
	let args = unsafe { &mut *(args as *mut InvokeArgs) };
	let inst = unsafe { &mut *inst_mut(vm_handle) };
	inst.set_gas(args.gas);
	for (reg, value) in Reg::ALL.into_iter().zip(args.regs.iter().copied()) {
		inst.set_reg(reg, value);
	}
	let kind = match inst.run() {
		Ok(kind) => kind,
		Err(error) => panic!("inner-PVM run failed: {error}"),
	};
	args.gas = inst.gas();
	for (reg, value) in Reg::ALL.into_iter().zip(args.regs.iter_mut()) {
		*value = inst.reg(reg);
	}
	match kind {
		InterruptKind::Finished => (InvokeOutcomeCode::Halt as u64, 0),
		InterruptKind::Ecalli(index) => (InvokeOutcomeCode::HostCallFault as u64, index as u64),
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
pub extern "C" fn expunge(vm_handle: u64) -> u64 {
	with_vm(|vm| {
		vm.instances.remove(&vm_handle);
		0
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
	_service: u64,
	hash_ptr: *const u8,
	out: *mut u8,
	offset: u64,
	out_len: u64,
) -> u64 {
	let mut hash = [0u8; 32];
	unsafe { std::ptr::copy_nonoverlapping(hash_ptr, hash.as_mut_ptr(), 32) };
	with_vm(|vm| match vm.preimages.get(&hash) {
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
		Err(_) => panic!("polkavm 0.36 failed to parse the frameless blob"),
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

	let outcome =
		std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&parsed, ParaId(0))));

	with_vm(|vm| {
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
