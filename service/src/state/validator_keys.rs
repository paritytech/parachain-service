//! Validator-key staging buffer (spec §3.1, §5.3), assembled chunk by chunk by
//! Asset Hub via `set_validator_keys` and flushed to JAM `designate` on `is_last`.

use crate::{
	constants::MAX_STAGED_VALIDATOR_KEYS,
	state,
	state::{StorageFull, Tag},
};
use bounded_collections::{BoundedVec, ConstU32};
use parachain_service_interface::types::ValidatorKey;

/// The staging buffer value: at most `CORE_COUNT * 3 = 1023` keys.
pub type StagedKeys = BoundedVec<ValidatorKey, ConstU32<{ MAX_STAGED_VALIDATOR_KEYS as u32 }>>;

/// Storage accessors for the `staged_validator_keys` singleton (tag `0x05`).
pub struct StagedValidatorKeys;

impl StagedValidatorKeys {
	pub fn get() -> StagedKeys {
		state::read_singleton(Tag::StagedValidatorKeys).unwrap_or_default()
	}

	/// Persist the staging buffer. `Err(StorageFull)` on the §6.1 backstop; see
	/// [`crate::state::write`].
	pub fn set(keys: &StagedKeys) -> Result<(), StorageFull> {
		state::write_singleton(Tag::StagedValidatorKeys, keys)
	}

	pub fn clear() {
		state::clear_singleton(Tag::StagedValidatorKeys)
	}
}
