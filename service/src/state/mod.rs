//! The Parachain Service's on-chain state (spec §3.1).
//!
//! All state mutations happen through Accumulate; Refine is stateless. Each
//! top-level storage item is assigned a 1-byte tag; the full JAM storage key is
//! `[tag] || SCALE(logical key)` (the tag alone for singletons).
//!
//! Every submodule pairs the value types with typed accessors over the JAM
//! `get_storage`/`set_storage` host calls. Pure logic (sizing, eviction) is kept
//! free of storage I/O so it can be unit-tested on the host.

pub mod assigns;
pub mod kv;
pub mod log;
pub mod para_info;
pub mod preimage_registry;
pub mod transfers;
pub mod validator_keys;

use alloc::vec::Vec;
use codec::{Decode, Encode};

/// Storage-item tags (spec §3.1, "Storage key encoding").
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
	Parachains = 0x00,
	ParachainLog = 0x01,
	PendingAssigns = 0x02,
	PendingAssignCores = 0x03,
	PreimageRegistry = 0x04,
	StagedValidatorKeys = 0x05,
	IncomingTransfers = 0x06,
	IncomingTransferChain = 0x07,
	KeyValueStorage = 0x08,
}

/// The full JAM storage key for a map entry: `[tag] || SCALE(key)`.
pub fn storage_key(tag: Tag, key: &impl Encode) -> Vec<u8> {
	let mut k = Vec::with_capacity(1 + key.encoded_size());
	k.push(tag as u8);
	key.encode_to(&mut k);
	k
}

/// Read + decode a map entry. `None` if absent.
///
/// Panics on undecodable stored data: state is only ever written by this
/// service, so a decode failure is a bug, not an input error.
pub fn read<V: Decode>(tag: Tag, key: &impl Encode) -> Option<V> {
	let raw = jam_pvm_common::accumulate::get_storage(&storage_key(tag, key))?;
	Some(V::decode(&mut &raw[..]).expect("service-written state must decode; qed"))
}

/// Encode + write a map entry.
///
/// Panics on `StorageFull`: every growth is pre-checked against
/// `total_state_balance` (§6.1 write-time invariant), so a JAM-level balance
/// failure indicates a bookkeeping bug.
/// FIXME: consensus-critical — a panic here reverts to the last checkpoint and
/// can wedge the service until manual intervention.
pub fn write(tag: Tag, key: &impl Encode, value: &impl Encode) {
	jam_pvm_common::accumulate::set_storage(&storage_key(tag, key), &value.encode())
		.expect("state growth is pre-checked against the state balance; qed");
}

/// Remove a map entry (no-op if absent).
pub fn clear(tag: Tag, key: &impl Encode) {
	let _ = jam_pvm_common::accumulate::remove_storage(&storage_key(tag, key));
}

/// Read + decode a singleton (the tag alone is the key).
pub fn read_singleton<V: Decode>(tag: Tag) -> Option<V> {
	read(tag, &())
}

/// Encode + write a singleton.
pub fn write_singleton(tag: Tag, value: &impl Encode) {
	write(tag, &(), value)
}

/// Remove a singleton.
pub fn clear_singleton(tag: Tag) {
	clear(tag, &())
}
