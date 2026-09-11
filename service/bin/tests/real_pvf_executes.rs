//! Proves the real `parachain-template-runtime` PolkaVM blob executes as a child PVF
//! under the service's real executor (`service/src/pvf/pvm.rs::run`), outside any JAM
//! network: parse (polkavm 0.36 vs the 0.35-linked blob), inner-PVM instantiation,
//! the `grow_heap` path that used to panic, and `jam_validate_block`'s read/declare
//! host-call surface.
//!
//! The service runs here as native Rust. Its child-PVM host calls (`machine`, `invoke`,
//! `peek`, `poke`, `pages`, `gas`, `expunge`, `export`, `fetch`, `historical_lookup`,
//! `log`) are JAM imports that only link inside a guest build; the shims below provide
//! them, backed by a real polkavm 0.36 `RawInstance` in thread-local state, mirroring
//! what polkajam's own `machine`/`invoke`/`pages` host calls do. Unlike the `grow_heap.rs`
//! shims (which panic on contact), these genuinely execute the child PVM.

use std::{cell::RefCell, collections::HashMap};

use jam_types::InvokeOutcomeCode;
use parachain_service::pvf::pvm::{parse_pvf, run};
use parachain_service_interface::types::ParaId;
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
	payload: Vec<u8>,
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
				payload: Vec::new(),
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
	_index: u64,
	_b: u64,
) -> u64 {
	// Only the work-item payload selector (kind 13, Gray Paper `workitems[a].payload`)
	// is served; anything else is absent (`u64::MAX`).
	if kind != 13 {
		return u64::MAX;
	}
	with_vm(|vm| {
		let payload = &vm.payload;
		let full = payload.len() as u64;
		if offset >= full {
			return full;
		}
		let n = (full - offset).min(buffer_len) as usize;
		unsafe { std::ptr::copy_nonoverlapping(payload.as_ptr().add(offset as usize), buffer, n) };
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

// --- Fixture: the canonical runtime blob -------------------------------------

const POLKAVM_BLOB: &str = match option_env!("POLKAVM_BLOB") {
	Some(path) => path,
	None => concat!(
		env!("CARGO_MANIFEST_DIR"),
		"/../../../polkadot-sdk3/.omo/evidence/",
		"jam-zombienet-real-service/parachain-template-runtime.polkavm"
	),
};
const POLKAVM_BLOB_LEN: usize = 7_004_302;

fn blob() -> Vec<u8> {
	let blob = std::fs::read(POLKAVM_BLOB)
		.expect("the canonical PolkaVM runtime blob must exist at the evidence path; qed");
	assert_eq!(blob.len(), POLKAVM_BLOB_LEN, "T2 recorded byte length");
	assert_eq!(&blob[..4], b"PVM\0", "the blob must be a PolkaVM program, not WASM");
	blob
}

// --- Minimal SCALE encoders ---------------------------------------------------

fn compact(n: u64) -> Vec<u8> {
	if n < 1 << 6 {
		vec![(n as u8) << 2]
	} else if n < 1 << 14 {
		vec![((n as u8) & 0x3f) << 2 | 0b01, (n >> 6) as u8]
	} else if n < 1 << 30 {
		let mut out = vec![((n as u8) & 0x3f) << 2 | 0b10];
		out.extend_from_slice(&(n >> 6).to_le_bytes()[..3]);
		out
	} else {
		let mut out = vec![0b11];
		out.extend_from_slice(&(n as u32).to_le_bytes());
		out
	}
}

fn bytes(data: &[u8]) -> Vec<u8> {
	let mut out = compact(data.len() as u64);
	out.extend_from_slice(data);
	out
}

fn blake2b_256(data: &[u8]) -> [u8; 32] {
	let mut out = [0u8; 32];
	out.copy_from_slice(blake2b_simd::Params::new().hash_length(32).hash(data).as_bytes());
	out
}

// --- The work-item payload the runtime's `jam_validate_block` reads -----------

/// SCALE-encoded `sp_runtime::generic::Header<u32, BlakeTwo256>` for the parent head.
///
/// `state_root` is the Blake2-256 empty trie root, so an empty `CompactProof` in the
/// block data verifies against it (`CompactProof::to_memory_db` checks the decoded
/// root against the parent head's state root) — the deepest a synthetic, relay-less
/// block reaches before `frame_executive::initial_checks` needs a real genesis state
/// (`block_hash(0)` must equal the parent head hash).
const EMPTY_TRIE_ROOT: [u8; 32] = [
	0x03, 0x17, 0x0a, 0x2e, 0x75, 0x97, 0xb7, 0xb7, 0xe3, 0xd8, 0x4c, 0x05, 0x39, 0x1d, 0x13, 0x9a,
	0x62, 0xb1, 0x57, 0xe7, 0x87, 0x86, 0xd8, 0xc0, 0x82, 0xf2, 0x9d, 0xcf, 0x4c, 0x11, 0x13, 0x14,
];

fn parent_head() -> Vec<u8> {
	let mut header = Vec::new();
	header.extend_from_slice(&[0u8; 32]); // parent_hash
	header.extend_from_slice(&0u32.to_le_bytes()); // number
	header.extend_from_slice(&EMPTY_TRIE_ROOT);
	header.extend_from_slice(&[0u8; 32]); // extrinsics_root
	header.push(0x00); // digest: empty
	header
}

/// SCALE-encoded `ParachainBlockData::V1` with one block and an empty compact proof.
fn block_data(parent_hash: [u8; 32], extrinsics: Vec<Vec<u8>>) -> Vec<u8> {
	// Block: Header ++ Vec<UncheckedExtrinsic>.
	let mut block = Vec::new();
	block.extend_from_slice(&parent_hash);
	block.extend_from_slice(&1u32.to_le_bytes()); // block number 1
	block.extend_from_slice(&[0u8; 32]); // state_root
	block.extend_from_slice(&[0u8; 32]); // extrinsics_root
	block.push(0x00); // digest: empty
	block.extend_from_slice(&compact(extrinsics.len() as u64));
	for uxt in &extrinsics {
		block.extend_from_slice(&bytes(uxt));
	}

	let mut data = vec![0x01]; // ParachainBlockData::V1
	data.extend_from_slice(&compact(1)); // one block
	data.extend_from_slice(&bytes(&block));
	data.push(0x00); // CompactProof: empty
	data
}

/// SCALE-encoded `MemoryOptimizedValidationParams` (extension `None`, V1/V2 path).
fn params(parent_head: Vec<u8>, block_data: Vec<u8>, relay_parent_number: u32) -> Vec<u8> {
	let mut p = bytes(&parent_head);
	p.extend_from_slice(&bytes(&block_data));
	p.extend_from_slice(&relay_parent_number.to_le_bytes());
	p.extend_from_slice(&[0u8; 32]); // relay_parent_storage_root
	p // extension: None encodes nothing
}

// --- The test ----------------------------------------------------------------

#[test]
fn real_runtime_executes_as_child_pvf() {
	let blob_bytes = blob();
	let parsed = match parse_pvf(&blob_bytes) {
		Ok(parsed) => parsed,
		Err(_) => panic!("polkavm 0.36 failed to parse the 0.35-linked blob"),
	};
	let parent = parent_head();
	let parent_hash = blake2b_256(&parent);
	let payload = params(parent.clone(), block_data(parent_hash, Vec::new()), 1);

	with_vm(|vm| {
		vm.payload = payload;
		vm.gas_remaining = 1_000_000_000;
		vm.preimages.insert(blake2b_256(&blob_bytes), blob_bytes.clone());
	});

	let outcome =
		std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&parsed, ParaId(0))));

	with_vm(|vm| {
		// The runtime's allocator (`sp_io` riscv `grow_heap`, import index 1) must have been
		// exercised — the handler that used to panic under the pre-real-PVF shims. Observed
		// through the executor's own `grow_heap probe` log line, so this proves the dispatch
		// genuinely ran the child PVM.
		let grew = vm.logs.iter().any(|line| line.contains("grow_heap probe:"));
		// `set_parent_head_hash` (import 100) is only reachable after the runtime decoded the
		// `MemoryOptimizedValidationParams` work-item payload and entered `jam_validate_block`.
		let declared = vm.logs.iter().any(|line| line.contains("dispatch probe: call=100"));
		println!("host-call log:\n{}", vm.logs.join("\n"));
		println!("grow_heap exercised: {grew}");
		println!("set_parent_head_hash reached: {declared}");
		println!("abnormal exit: {:?}", vm.trap);
		assert!(grew, "the runtime's allocator must call `grow_heap` during validation");
		assert!(
			declared,
			"the runtime must decode the work-item payload and reach jam_validate_block"
		);
	});

	// The work-item input is a synthetic, relay-less block, so full block validation is not
	// expected to complete: the runtime executes and reaches block execution, then stops at a
	// guest panic. A completed `jam_validate_block` (head set) would be the e2e success; the
	// abnormal-exit report below is the residual the owner asked to see.
	match outcome {
		Ok(Ok((_parent_head_hash, head, _umps))) => {
			assert!(!head.as_slice().is_empty(), "the runtime set a non-empty head");
			println!("jam_validate_block COMPLETED: head_len={}", head.as_slice().len());
		},
		Ok(Err(err)) => {
			eprintln!("jam_validate_block returned RefineLog::{err:?}");
		},
		Err(panic) => {
			let msg = panic
				.downcast_ref::<String>()
				.cloned()
				.or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
				.unwrap_or_else(|| "<non-string panic>".to_owned());
			with_vm(|vm| {
				eprintln!(
					"the runtime executed but the synthetic block was rejected:\n  panic: {msg}\n  trap: {:?}",
					vm.trap
				);
			});
		},
	}
}
