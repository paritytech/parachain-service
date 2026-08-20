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

/// Marker returned when a state write is rejected by JAM with `StorageFull`.
///
/// Per the §6.1 write-time invariant every growth is pre-checked against
/// `total_state_balance`, so a JAM-level balance failure here means the private
/// headroom accounting diverged from the real service balance (SPEC_GAPS #4).
/// The failure is surfaced rather than panicked: the accumulate growth paths
/// log `AccumulateLog::InsufficientStateBalance` and continue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageFull;

/// Encode + write a map entry.
///
/// Returns `Err(StorageFull)` on a JAM-level balance failure instead of
/// panicking. The §6.1 pre-check makes this unreachable in practice, but a
/// panic would revert to the last checkpoint and wedge the service until
/// manual intervention.
/// FIXME: consensus-critical — private headroom ≠ real JAM balance (SPEC_GAPS #4).
pub fn write(tag: Tag, key: &impl Encode, value: &impl Encode) -> Result<(), StorageFull> {
	jam_pvm_common::accumulate::set_storage(&storage_key(tag, key), &value.encode())
		.map(|_| ())
		.map_err(|_| StorageFull)
}

/// Remove a map entry (no-op if absent).
pub fn clear(tag: Tag, key: &impl Encode) {
	let _ = jam_pvm_common::accumulate::remove_storage(&storage_key(tag, key));
}

/// Read + decode a singleton (the tag alone is the key).
pub fn read_singleton<V: Decode>(tag: Tag) -> Option<V> {
	read(tag, &())
}

/// Encode + write a singleton. See [`write`] for the failure semantics.
pub fn write_singleton(tag: Tag, value: &impl Encode) -> Result<(), StorageFull> {
	write(tag, &(), value)
}

/// Remove a singleton.
pub fn clear_singleton(tag: Tag) {
	clear(tag, &())
}
