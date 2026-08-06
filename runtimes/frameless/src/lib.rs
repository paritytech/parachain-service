//! Minimal FRAME-less mock runtime for both chains the service targets.
//!
//! A [`Config`] fixed at genesis and carried in [`State`] selects Coretime or Asset
//! Hub; no block can change it. Modeled on polkadot-sdk's `adder` test parachain: the
//! state is a `u64` counter, each block applies a per-[`Config`] transition, and the
//! runtime is one [`execute`] step behind the [`validate_block`] PVM entry point.
//!
//! The guest's global allocator is a fixed-arena bump allocator (see the
//! `arena_allocator` module). JAM's `jam_v1` ISA has no `sbrk`, so a growable in-guest
//! heap (picoalloc / RFC-145) can't link; sp-io's own host-call allocator is turned off
//! via its `disable_allocator` feature, and sp-io is kept only for its panic/OOM
//! handlers. `substrate-wasm-builder` builds with `--cfg substrate_runtime`, which drops
//! sp-io's `secp256k1` C sources — so, unlike the bare JAM guests, this runtime can
//! depend on sp-io.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

// Guest-only fixed-arena `#[global_allocator]`. sp-io's host-call allocator is disabled
// (its `disable_allocator` feature), so this is the only one.
#[cfg(target_arch = "riscv64")]
mod arena_allocator;

// Keep sp-io linked (never called directly) for the guest's panic/OOM handlers; std
// provides those on the host build.
#[cfg(not(feature = "std"))]
use sp_io as _;

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

/// Which parachain this is. Fixed at genesis and carried in [`State`]; a block cannot
/// change it, since its start state must hash-match the parent head and
/// [`State::transition`] keeps the `Config`.
#[derive(Clone, Copy, Eq, PartialEq, Encode, Decode, Debug)]
pub enum Config {
	/// The Coretime chain.
	Coretime,
	/// The Asset Hub chain.
	AssetHub,
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
	fn transition(&self, add: u64) -> State {
		let counter = match self.config {
			// FIXME: Coretime and Asset Hub logic (host calls, hooks) will diverge here;
			// for now both just wrapping-add like `adder`.
			Config::Coretime => self.counter.wrapping_add(add),
			Config::AssetHub => self.counter.wrapping_add(add),
		};
		State { config: self.config, counter }
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

/// Input to [`validate_block`]. Parent head and block body as opaque SCALE bytes, per
/// `adder`'s wire format (minus its relay-chain fields, which JAM has no use for).
#[derive(Clone, Encode, Decode, Debug)]
pub struct ValidationParams {
	/// SCALE-encoded parent [`HeadData`].
	pub parent_head: Vec<u8>,
	/// SCALE-encoded [`BlockData`].
	pub block_data: Vec<u8>,
}

/// Host-testable core of [`validate_block`]: decode [`ValidationParams`], [`execute`],
/// encode the new [`HeadData`]. Panics on bad input, as the entry point does.
pub fn validate(input: &[u8]) -> Vec<u8> {
	let params = ValidationParams::decode(&mut &input[..]).expect("invalid validation params");

	let parent_head = HeadData::decode(&mut &params.parent_head[..]).expect("invalid parent head");
	let block_data = BlockData::decode(&mut &params.block_data[..]).expect("invalid block data");

	let new_head = execute(parent_head, &block_data).expect("block is valid");

	new_head.encode()
}

/// PVM entry point: validate one block.
///
/// The host passes the `(ptr, len)` of a SCALE-encoded [`ValidationParams`] in guest
/// memory; we return the `(ptr, len)` of the encoded new [`HeadData`], leaked so it
/// outlives the call. Logic is in [`validate`]; this is only the memory marshalling.
#[cfg(target_arch = "riscv64")]
#[polkavm_derive::polkavm_export]
extern "C" fn validate_block(ptr: u32, len: u32) -> (u64, u64) {
	// The refine service poked a SCALE-encoded `ValidationParams` at `ptr`; decode it,
	// validate the block, and hand back the `(ptr, len)` of the encoded new `HeadData`,
	// leaked so it outlives the call.
	let input = unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
	let output = alloc::boxed::Box::leak(validate(input).into_boxed_slice());
	(output.as_ptr() as u64, output.len() as u64)
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
