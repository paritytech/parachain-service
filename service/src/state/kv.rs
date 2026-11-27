//! Per-parachain key/value store (spec §3.1, §6.1), keyed `(ParaId, user_key)`.

use crate::state::{StorageFull, Tag};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::{remove_storage, set_storage};
use parachain_service_core::types::ParaId;

/// The JAM storage key of `key_value_storage[(para_id, key)]`: `0x08 || para_id || key`,
/// with the user key appended as sent, without a length prefix (§3.1).
pub fn storage_key(para_id: ParaId, key: &[u8]) -> Vec<u8> {
	let prefix = Tag::KeyValueStorage.para_key(para_id);
	let mut result = Vec::with_capacity(prefix.len() + key.len());
	result.extend_from_slice(&prefix);
	result.extend_from_slice(key);
	result
}

fn with_storage_key<R>(para_id: ParaId, key: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
	let prefix = Tag::KeyValueStorage.para_key(para_id);
	if key.len() <= 32 {
		let mut bytes = [0u8; 37];
		bytes[..5].copy_from_slice(&prefix);
		bytes[5..5 + key.len()].copy_from_slice(key);
		f(&bytes[..5 + key.len()])
	} else {
		f(&storage_key(para_id, key))
	}
}

/// Storage accessors for the `key_value_storage` map (tag `0x08`). Values are
/// stored as sent, without a length prefix (§6.1).
pub struct KeyValueStorage;

impl KeyValueStorage {
	/// The length of the stored value, without copying it: JAM's `read` returns
	/// the full length whatever the output length (§6.1).
	pub fn value_len(para_id: ParaId, key: &[u8]) -> Option<usize> {
		with_storage_key(para_id, key, |key| {
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
		})
	}

	/// Upsert a value. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(para_id: ParaId, key: &[u8], value: &[u8]) -> Result<(), StorageFull> {
		with_storage_key(para_id, key, |key| {
			set_storage(key, value).map(|_| ()).map_err(|_| StorageFull)
		})
	}

	/// Remove an entry, returning the length of the removed value.
	pub fn remove(para_id: ParaId, key: &[u8]) -> Option<usize> {
		with_storage_key(para_id, key, remove_storage)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn key_layout_matches_for_short_and_long_keys() {
		let para = ParaId::from(0x1234_5678);
		for key in [&[1u8, 2][..], &[3u8; 33][..]] {
			with_storage_key(para, key, |actual| {
				let mut expected = alloc::vec![0x08, 0x78, 0x56, 0x34, 0x12];
				expected.extend_from_slice(key);
				assert_eq!(actual, expected);
				assert_eq!(actual, storage_key(para, key));
			});
		}
	}
}
