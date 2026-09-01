//! Per-parachain key/value store (spec §3.1, §6.1), keyed `(ParaId, user_key)`.

use crate::state::{self, Tag};
use alloc::vec::Vec;
use parachain_service_interface::types::ParaId;

/// Storage accessors for the `key_value_storage` map (tag `0x08`).
///
/// Key encoding: `para_id` (4 B) then the SCALE `Vec<u8>` user key
/// (compact length prefix + bytes), matching the §6.1 `kv_entry_footprint`.
pub struct KeyValueStorage;

impl KeyValueStorage {
	pub fn get(para_id: ParaId, key: &[u8]) -> Option<Vec<u8>> {
		state::read(Tag::KeyValueStorage, &(para_id, key))
	}

	pub fn set(para_id: ParaId, key: &[u8], value: &[u8]) {
		state::write(Tag::KeyValueStorage, &(para_id, key), &value)
	}

	pub fn remove(para_id: ParaId, key: &[u8]) {
		state::clear(Tag::KeyValueStorage, &(para_id, key))
	}
}
