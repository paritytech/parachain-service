//! Per-parachain key/value store (spec §3.1, §6.1), keyed `(ParaId, user_key)`.

use crate::state::{StorageFull, Tag};
use alloc::vec::Vec;
use codec::Encode;
use jam_pvm_common::accumulate::{remove_storage, set_storage};
use parachain_service_core::types::ParaId;

/// The JAM storage key of `key_value_storage[(para_id, key)]`: `0x08 || para_id || key`,
/// with the user key appended as sent, without a length prefix (§3.1).
pub fn storage_key(para_id: ParaId, key: &[u8]) -> Vec<u8> {
	[&[Tag::KeyValueStorage as u8][..], &para_id.encode(), key].concat()
}

/// Storage accessors for the `key_value_storage` map (tag `0x08`). Values are
/// stored as sent, without a length prefix (§6.1).
pub struct KeyValueStorage;

impl KeyValueStorage {
	/// The length of the stored value, without copying it: JAM's `read` returns
	/// the full length whatever the output length (§6.1).
	pub fn value_len(para_id: ParaId, key: &[u8]) -> Option<usize> {
		let key = storage_key(para_id, key);
		// `jam_pvm_common` does not export `get_storage_into`, so call `read` directly.
		let len = unsafe {
			jam_pvm_common::imports::read(
				u64::MAX,
				key.as_ptr(),
				key.len() as u64,
				[0u8; 0].as_mut_ptr(),
				0,
				0,
			)
		};
		// `u64::MAX` is JAM's `NONE`: there is no entry.
		(len != u64::MAX).then_some(len as usize)
	}

	/// Upsert a value. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(para_id: ParaId, key: &[u8], value: &[u8]) -> Result<(), StorageFull> {
		set_storage(&storage_key(para_id, key), value)
			.map(|_| ())
			.map_err(|_| StorageFull)
	}

	/// Remove an entry, returning the length of the removed value.
	pub fn remove(para_id: ParaId, key: &[u8]) -> Option<usize> {
		remove_storage(&storage_key(para_id, key))
	}
}
