//! Minimal FRAME-less mock runtime for both chains the service targets.
//!
//! A [`Config`] fixed at genesis and carried in [`State`] selects Coretime or Asset
//! Hub; no block can change it. Modeled on polkadot-sdk's `adder` test parachain: the
//! state is a `u64` counter, each block applies a per-[`Config`] transition, and the
//! runtime is one [`execute`] step behind the [`jam_validate_block`] PVM entry point.
//!
//! The guest's global allocator is a fixed-arena bump allocator (see the
//! `arena_allocator` module). JAM's `jam_v1` ISA has no `sbrk`, so a growable in-guest
//! heap (picoalloc / RFC-145) can't link; sp-io's own host-call allocator is turned off
//! via its `disable_allocator` feature. Its `#[panic_handler]` is likewise disabled
//! (`disable_panic_handler`) — it reaches the `logging::log` host import, which, being
//! un-indexed, clashes with this runtime's explicitly-indexed host calls under
//! polkavm-linker's all-or-none index rule — and replaced by the trapping handler below.
//! sp-io stays a dependency for future host-function use. `substrate-wasm-builder` builds
//! with `--cfg substrate_runtime`, which drops sp-io's `secp256k1` C sources — so, unlike
//! the bare JAM guests, this runtime can depend on sp-io.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Guest-only fixed-arena `#[global_allocator]`. sp-io's host-call allocator is disabled
// (its `disable_allocator` feature), so this is the only one.
#[cfg(target_arch = "riscv64")]
mod arena_allocator;

// Keep sp-io in the dependency graph for future host-function use; it is not called and,
// with its panic handler and allocator disabled (see Cargo.toml + module docs),
// contributes no linked code. The `as _` binding only silences the unused-dep lint.
#[cfg(not(feature = "std"))]
use sp_io as _;

// Guest panic handler: trap the PVM directly. sp-io's own `#[panic_handler]` is disabled
// (its `disable_panic_handler` feature) because it reaches the un-indexed `logging::log`
// host import, which is incompatible with this runtime's explicitly-indexed host calls. A
// panic fails the candidate regardless, so trapping without a host round-trip is
// equivalent; the host `std` build supplies its own panic handler.
#[cfg(target_arch = "riscv64")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
	// The same trap instruction sp-io's `unreachable()` emits on RISC-V.
	unsafe { core::arch::asm!("unimp", options(noreturn)) }
}

// The runtime blob `substrate-wasm-builder` embeds for the host build:
// `WASM_BINARY: Option<&[u8]>`. Absent in the guest build.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

use alloc::vec::Vec;
use codec::{Decode, Encode};
use tiny_keccak::{Hasher as _, Keccak};

fn keccak256(input: &[u8]) -> [u8; 32] {
	let mut out = [0u8; 32];
	let mut hasher = Keccak::v256();
	hasher.update(input);
	hasher.finalize(&mut out);
	out
}

/// blake2b-256 — the hash the Parachain Service applies to stored head data, so
/// `set_parent_head_hash` must use it too (DECISIONS.md D-5). Distinct from this
/// chain's *internal* keccak head hashing, which the service never sees.
pub fn blake2_256(input: &[u8]) -> [u8; 32] {
	let hash = blake2b_simd::Params::new().hash_length(32).hash(input);
	hash.as_bytes().try_into().expect("hash_length(32) yields 32 bytes; qed")
}

/// Which parachain this is. Fixed at genesis and carried in [`State`]; a block cannot
/// change it, since its start state must hash-match the parent head and
/// [`State::transition`] keeps the `Config`.
#[derive(Clone, Eq, PartialEq, Encode, Decode, Debug)]
pub enum Config {
	/// The Coretime chain.
	Coretime,
	/// The Asset Hub chain.
	AssetHub,
	Mock(Vec<MockAction>),
}

/// One scripted side-effect host call, driven from the block body in tests via
/// [`Config::Mock`]. Mirrors the §4.3 side-effect host functions with primitive
/// field types so the test crate composes them freely.
#[derive(Clone, Encode, Decode, Debug, PartialEq, Eq)]
pub enum MockAction {
	KVSet(Vec<u8>, Vec<u8>),
	KVRemove {
		para_id: u32,
		key: Vec<u8>,
	},
	Solicit {
		target: parachain_service_interface::upward_message::Target,
		hash: [u8; 32],
		len: u32,
	},
	EjectService {
		service: u32,
	},
	SetServiceSupervisor {
		service: u32,
		new_supervisor: u32,
	},
	CreateService(parachain_service_interface::upward_message::CreateServiceArgs),
	Forget {
		target: parachain_service_interface::upward_message::Target,
		hash: [u8; 32],
		len: u32,
	},
	RemoveServiceStorage {
		service: u32,
		key: Vec<u8>,
	},
	RequestCodeUpgrade {
		hash: [u8; 32],
		len: u32,
	},
	TransferOut(parachain_service_interface::upward_message::TransferOutArgs),
	AssignCore {
		core: u32,
		queue: Vec<[u8; 32]>,
		assigner: Option<u32>,
		jam_slot: u32,
	},
	/// `keys` is the raw concatenation of 336-byte validator keys.
	SetValidatorKeys {
		keys: Vec<u8>,
		is_last: bool,
	},
	ConsumeTransfersUpTo {
		slot: u32,
	},
	ServiceUpgrade {
		code_hash: [u8; 32],
		len: u32,
		min_acc_gas: u64,
		min_memo_gas: u64,
	},
	/// Aborts the PVF via `report_error` — nothing after it executes.
	ReportError(Vec<u8>),
	ParachainSetHead {
		para_id: u32,
		head: Vec<u8>,
	},
	ParachainSetValidationCode {
		para_id: u32,
		hash: [u8; 32],
		len: u32,
	},
	ParachainCleanUp {
		para_id: u32,
	},
	ParachainSetStateBalance {
		para_id: u32,
		total: u64,
	},
	/// Calls `set_head` with an extra out-of-band declaration (ABI-violation test).
	DuplicateSetHead(Vec<u8>),
	/// Suppresses the entry point's own mandatory head declarations
	/// (ABI-violation test: exits without `set_head`/`set_parent_head_hash`).
	SkipHeadDeclarations,
}

/// The full chain state: the immutable [`Config`] plus the mutable counter.
#[derive(Clone, Eq, PartialEq, Encode, Decode, Debug)]
pub struct State {
	/// Which parachain this is. Set at genesis, never changed by a block.
	pub config: Config,
	/// The running counter, mutated by each block.
	pub counter: u64,
}

impl State {
	/// Apply one block's `add`, carrying [`State::config`] through unchanged.
	/// [`Config::Mock`] actions fire their scripted host calls here, in order.
	fn transition(&self, add: u64) -> State {
		if let Config::Mock(actions) = &self.config {
			for action in actions {
				#[cfg(target_arch = "riscv64")]
				host::run_action(action);
				#[cfg(not(target_arch = "riscv64"))]
				let _ = action;
			}
		}

		let counter = self.counter.wrapping_add(add);
		State { config: self.config.clone(), counter } // TODO remove clone
	}
}

/// Head data for this parachain.
#[derive(Default, Clone, Hash, Eq, PartialEq, Encode, Decode, Debug)]
pub struct HeadData {
	/// Block number.
	pub number: u64,
	/// keccak256 of the parent's head data.
	pub parent_hash: [u8; 32],
	/// keccak256 of the post-execution [`State`].
	pub post_state: [u8; 32],
}

impl HeadData {
	pub fn hash(&self) -> [u8; 32] {
		keccak256(&self.encode())
	}
}

/// Block body for this parachain.
#[derive(Clone, Encode, Decode, Debug)]
pub struct BlockData {
	/// State to begin from.
	pub state: State,
	/// Amount to add (wrapping).
	pub add: u64,
}

/// Hash a [`State`] the same way [`HeadData::post_state`] commits to it.
pub fn hash_state(state: &State) -> [u8; 32] {
	keccak256(&state.encode())
}

/// The block's start state did not match the parent head's committed state.
#[derive(Debug)]
pub struct StateMismatch;

/// Execute one block on top of `parent_head`, producing the new head data if valid.
pub fn execute(parent_head: HeadData, block_data: &BlockData) -> Result<HeadData, StateMismatch> {
	if hash_state(&block_data.state) != parent_head.post_state {
		return Err(StateMismatch);
	}

	let new_state = block_data.state.transition(block_data.add);

	Ok(HeadData {
		number: parent_head.number.checked_add(1).expect("block number overflow"),
		parent_hash: parent_head.hash(),
		post_state: hash_state(&new_state),
	})
}

/// Input to [`jam_validate_block`]. Parent head and block body as opaque SCALE bytes, per
/// `adder`'s wire format (minus its relay-chain fields, which JAM has no use for).
#[derive(Clone, Encode, Decode, Debug)]
pub struct ValidationParams {
	/// SCALE-encoded parent [`HeadData`].
	pub parent_head: Vec<u8>,
	/// SCALE-encoded [`BlockData`].
	pub block_data: Vec<u8>,
}

/// Host-testable core of [`jam_validate_block`]: decode [`ValidationParams`], [`execute`],
/// encode the new [`HeadData`]. Panics on bad input, as the entry point does.
pub fn validate(input: &[u8]) -> Vec<u8> {
	let params = ValidationParams::decode(&mut &input[..]).expect("invalid validation params");

	let parent_head = HeadData::decode(&mut &params.parent_head[..]).expect("invalid parent head");
	let block_data = BlockData::decode(&mut &params.block_data[..]).expect("invalid block data");

	let new_head = execute(parent_head, &block_data).expect("block is valid");

	new_head.encode()
}

/// PVM entry point: validate one block (DECISIONS.md D-1).
///
/// Reads inputs via `work_item_payload(0)` (spec §4.2). Results are declared
/// exclusively through the mandatory `set_parent_head_hash` + `set_head` host
/// calls; nothing is returned through registers.
#[cfg(target_arch = "riscv64")]
#[polkavm_derive::polkavm_export]
extern "C" fn jam_validate_block() {
	use parachain_service_interface::candidate::ParachainCandidate;
	// index 0: service/src/refine.rs panics unless the package has exactly one item.
	let raw = host::work_item_payload(0).expect("work item payload present; qed");
	let candidate =
		ParachainCandidate::decode(&mut &raw[..]).expect("ParachainCandidate decodes; qed");
	let params =
		ValidationParams::decode(&mut &candidate.pov[..]).expect("invalid validation params");

	let parent_head = HeadData::decode(&mut &params.parent_head[..]).expect("invalid parent head");
	let block_data = BlockData::decode(&mut &params.block_data[..]).expect("invalid block data");

	// Mock actions fire inside the transition, in scripted order.
	let new_head = execute(parent_head, &block_data).expect("block is valid");

	if let Config::Mock(actions) = &block_data.state.config {
		if actions.iter().any(|a| matches!(a, MockAction::SkipHeadDeclarations)) {
			return;
		}
		for action in actions {
			if let MockAction::DuplicateSetHead(head) = action {
				host::set_head(head);
			}
		}
	}

	// The parent-head hash is over the encoded parent head data, with the hash
	// function the service applies to stored head data (D-5).
	host::set_parent_head_hash(&blake2_256(&params.parent_head));
	host::set_head(&new_head.encode());
}

/// Child host calls of the Parachain Service's Refine, per the ABI in
/// `parachain-service`'s `pvf/executor.rs` (indices from
/// `parachain-service-interface`'s `HostCall`).
#[cfg(target_arch = "riscv64")]
mod host {
	use super::MockAction;
	use alloc::vec::Vec;
	use codec::Encode;

	#[polkavm_derive::polkavm_import]
	extern "C" {
		// --- Data access ---
		#[polkavm_import(index = 0)]
		fn lookup_raw(hash_ptr: u32, out_ptr: u32, out_cap: u32) -> u64;
		#[polkavm_import(index = 2)]
		fn gas_raw() -> u64;
		#[polkavm_import(index = 9)]
		fn work_item_payload_raw(index: u32, out_ptr: u32, out_cap: u32) -> u64;
		// --- Side effects ---
		#[polkavm_import(index = 11)]
		fn export_raw(ptr: u32, len: u32) -> u64;
		#[polkavm_import(index = 12)]
		fn set_parent_head_hash_raw(hash_ptr: u32);
		#[polkavm_import(index = 13)]
		fn set_head_raw(ptr: u32, len: u32);
		#[polkavm_import(index = 14)]
		fn request_code_upgrade_raw(hash_ptr: u32, len: u32);
		#[polkavm_import(index = 15)]
		fn solicit_raw(target_kind: u32, target_id: u32, hash_ptr: u32, len: u32);
		#[polkavm_import(index = 16)]
		fn forget_raw(target_kind: u32, target_id: u32, hash_ptr: u32, len: u32);
		#[polkavm_import(index = 17)]
		fn kv_set_raw(key_ptr: u32, key_len: u32, value_ptr: u32, value_len: u32);
		#[polkavm_import(index = 18)]
		fn kv_remove_raw(para_id: u32, key_ptr: u32, key_len: u32);
		#[polkavm_import(index = 19)]
		fn transfer_out_raw(args_ptr: u32, args_len: u32);
		#[polkavm_import(index = 20)]
		fn assign_core_raw(
			core: u32,
			queue_ptr: u32,
			queue_count: u32,
			has_assigner: u32,
			assigner: u32,
			jam_slot: u32,
		);
		#[polkavm_import(index = 21)]
		fn set_validator_keys_raw(keys_ptr: u32, count: u32, is_last: u32);
		#[polkavm_import(index = 22)]
		fn consume_transfers_up_to_raw(slot: u32);
		#[polkavm_import(index = 23)]
		fn parachain_service_upgrade_raw(
			hash_ptr: u32,
			len: u32,
			min_acc_gas: u64,
			min_memo_gas: u64,
		);
		#[polkavm_import(index = 24)]
		fn report_error_raw(ptr: u32, len: u32);
		#[polkavm_import(index = 25)]
		fn parachain_set_head_raw(para_id: u32, head_ptr: u32, head_len: u32);
		#[polkavm_import(index = 26)]
		fn parachain_set_validation_code_raw(para_id: u32, hash_ptr: u32, len: u32);
		#[polkavm_import(index = 27)]
		fn parachain_clean_up_raw(para_id: u32);
		#[polkavm_import(index = 28)]
		fn parachain_set_state_balance_raw(para_id: u32, total: u64);
		#[polkavm_import(index = 29)]
		fn remove_service_storage_raw(service: u32, key_ptr: u32, key_len: u32);
		#[polkavm_import(index = 30)]
		fn eject_service_raw(service: u32);
		#[polkavm_import(index = 31)]
		fn set_service_supervisor_raw(service: u32, new_supervisor: u32);
		#[polkavm_import(index = 32)]
		fn create_service_raw(args_ptr: u32, args_len: u32);
	}

	pub fn set_parent_head_hash(hash: &[u8; 32]) {
		unsafe { set_parent_head_hash_raw(hash.as_ptr() as u32) }
	}

	pub fn set_head(head: &[u8]) {
		unsafe { set_head_raw(head.as_ptr() as u32, head.len() as u32) }
	}

	/// Fetch a preimage into a fresh buffer; `None` if unavailable.
	#[allow(dead_code)]
	pub fn lookup(hash: &[u8; 32], max_len: usize) -> Option<Vec<u8>> {
		let mut out = alloc::vec![0u8; max_len];
		let len = unsafe { lookup_raw(hash.as_ptr() as u32, out.as_ptr() as u32, max_len as u32) };
		if len == u64::MAX {
			return None;
		}
		out.truncate((len as usize).min(max_len));
		Some(out)
	}

	#[allow(dead_code)]
	pub fn gas() -> u64 {
		unsafe { gas_raw() }
	}

	/// Fetch work-item payload at `index`; `None` if absent.
	/// Probes with zero capacity to get the byte length, then retries with a
	/// large enough buffer (buffer protocol per `executor.rs` module docs).
	pub fn work_item_payload(index: u32) -> Option<Vec<u8>> {
		let len = unsafe { work_item_payload_raw(index, 0, 0) };
		if len == u64::MAX {
			return None;
		}
		let mut buf = alloc::vec![0u8; len as usize];
		loop {
			let actual =
				unsafe { work_item_payload_raw(index, buf.as_ptr() as u32, buf.len() as u32) };
			if actual == u64::MAX {
				return None;
			}
			let actual = actual as usize;
			if actual <= buf.len() {
				buf.truncate(actual);
				return Some(buf);
			}
			buf.resize(actual, 0);
		}
	}

	#[allow(dead_code)]
	pub fn export(data: &[u8]) -> u64 {
		unsafe { export_raw(data.as_ptr() as u32, data.len() as u32) }
	}

	/// The `(kind, id)` register pair a `Target` is passed as; must agree with
	/// `peek_target` in the service's `pvf::executor`.
	fn target_regs(target: &parachain_service_interface::upward_message::Target) -> (u32, u32) {
		use parachain_service_interface::upward_message::Target;
		match target {
			Target::Parachain(para_id) => (0, para_id.0),
			Target::Service(service) => (1, *service),
		}
	}

	/// Fire one scripted side-effect host call. Entry-point-level actions
	/// (`DuplicateSetHead`, `SkipHeadDeclarations`) are handled by
	/// `jam_validate_block` itself and are no-ops here.
	pub fn run_action(action: &MockAction) {
		match action {
			MockAction::KVSet(key, value) => unsafe {
				kv_set_raw(
					key.as_ptr() as u32,
					key.len() as u32,
					value.as_ptr() as u32,
					value.len() as u32,
				)
			},
			MockAction::KVRemove { para_id, key } => unsafe {
				kv_remove_raw(*para_id, key.as_ptr() as u32, key.len() as u32)
			},
			MockAction::Solicit { target, hash, len } => {
				let (kind, id) = target_regs(target);
				unsafe { solicit_raw(kind, id, hash.as_ptr() as u32, *len) }
			},
			MockAction::EjectService { service } => unsafe { eject_service_raw(*service) },
			MockAction::SetServiceSupervisor { service, new_supervisor } => unsafe {
				set_service_supervisor_raw(*service, *new_supervisor)
			},
			MockAction::CreateService(args) => {
				let encoded = args.encode();
				unsafe { create_service_raw(encoded.as_ptr() as u32, encoded.len() as u32) }
			},
			MockAction::Forget { target, hash, len } => {
				let (kind, id) = target_regs(target);
				unsafe { forget_raw(kind, id, hash.as_ptr() as u32, *len) }
			},
			MockAction::RemoveServiceStorage { service, key } => unsafe {
				remove_service_storage_raw(*service, key.as_ptr() as u32, key.len() as u32)
			},
			MockAction::RequestCodeUpgrade { hash, len } => unsafe {
				request_code_upgrade_raw(hash.as_ptr() as u32, *len)
			},
			MockAction::TransferOut(args) => {
				let encoded = args.encode();
				unsafe { transfer_out_raw(encoded.as_ptr() as u32, encoded.len() as u32) }
			},
			MockAction::AssignCore { core, queue, assigner, jam_slot } => unsafe {
				assign_core_raw(
					*core,
					queue.as_ptr() as u32,
					queue.len() as u32,
					assigner.is_some() as u32,
					assigner.unwrap_or(0),
					*jam_slot,
				)
			},
			MockAction::SetValidatorKeys { keys, is_last } => unsafe {
				set_validator_keys_raw(
					keys.as_ptr() as u32,
					(keys.len() / 336) as u32,
					*is_last as u32,
				)
			},
			MockAction::ConsumeTransfersUpTo { slot } => unsafe {
				consume_transfers_up_to_raw(*slot)
			},
			MockAction::ServiceUpgrade { code_hash, len, min_acc_gas, min_memo_gas } => unsafe {
				parachain_service_upgrade_raw(
					code_hash.as_ptr() as u32,
					*len,
					*min_acc_gas,
					*min_memo_gas,
				)
			},
			MockAction::ReportError(data) => unsafe {
				// Aborts the PVF host-side; execution never resumes.
				report_error_raw(data.as_ptr() as u32, data.len() as u32)
			},
			MockAction::ParachainSetHead { para_id, head } => unsafe {
				parachain_set_head_raw(*para_id, head.as_ptr() as u32, head.len() as u32)
			},
			MockAction::ParachainSetValidationCode { para_id, hash, len } => unsafe {
				parachain_set_validation_code_raw(*para_id, hash.as_ptr() as u32, *len)
			},
			MockAction::ParachainCleanUp { para_id } => unsafe { parachain_clean_up_raw(*para_id) },
			MockAction::ParachainSetStateBalance { para_id, total } => unsafe {
				parachain_set_state_balance_raw(*para_id, *total)
			},
			MockAction::DuplicateSetHead(_) | MockAction::SkipHeadDeclarations => {},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Genesis head for the given [`Config`], committing to a zero counter.
	fn genesis(config: Config) -> HeadData {
		HeadData {
			number: 0,
			parent_hash: [0; 32],
			post_state: hash_state(&State { config, counter: 0 }),
		}
	}

	#[test]
	fn coretime_works() {
		let parent = genesis(Config::Coretime);
		let block = BlockData { state: State { config: Config::Coretime, counter: 0 }, add: 512 };

		let new_head = execute(parent.clone(), &block).unwrap();

		assert_eq!(new_head.number, 1);
		assert_eq!(new_head.parent_hash, parent.hash());
		assert_eq!(
			new_head.post_state,
			hash_state(&State { config: Config::Coretime, counter: 512 })
		);
	}

	#[test]
	fn asset_hub_works() {
		let parent = genesis(Config::AssetHub);
		let block = BlockData { state: State { config: Config::AssetHub, counter: 0 }, add: 512 };

		let new_head = execute(parent.clone(), &block).unwrap();

		assert_eq!(new_head.number, 1);
		assert_eq!(new_head.parent_hash, parent.hash());
		assert_eq!(
			new_head.post_state,
			hash_state(&State { config: Config::AssetHub, counter: 512 })
		);
	}

	#[test]
	fn state_mismatch_errors() {
		let parent = genesis(Config::Coretime);
		// Start counter doesn't match the parent's commitment.
		let block = BlockData { state: State { config: Config::Coretime, counter: 999 }, add: 1 };

		assert!(execute(parent, &block).is_err());
	}

	#[test]
	fn validate_works() {
		// The wire path (encode params, validate, decode head) matches a direct `execute`.
		let parent = genesis(Config::AssetHub);
		let block = BlockData { state: State { config: Config::AssetHub, counter: 0 }, add: 2 };
		let params = ValidationParams { parent_head: parent.encode(), block_data: block.encode() };

		let head = HeadData::decode(&mut &validate(&params.encode())[..]).unwrap();

		assert_eq!(head, execute(parent, &block).unwrap());
	}

	#[test]
	fn config_change_errors() {
		// A block can't switch chains: an Asset Hub start state no longer hashes to the
		// parent's Coretime commitment.
		let parent = genesis(Config::Coretime);
		let block = BlockData { state: State { config: Config::AssetHub, counter: 0 }, add: 1 };

		assert!(execute(parent, &block).is_err());
	}
}
