//! Service storage key layout and per-parachain metadata (`ParaInfo`).

use crate::types::{Balance, HeadData, ParaId, ValidationCodeRef};
use codec::{Decode, Encode};

/// Storage-item tags (spec §3.1).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKey {
	Parachains = 0x00,
	ParachainLog = 0x01,
	PendingAssigns = 0x02,
	PendingAssignCores = 0x03,
	PreimageRegistry = 0x04,
	StagedValidatorKeys = 0x05,
	IncomingTransfers = 0x06,
	IncomingTransferBuckets = 0x07,
	KeyValueStorage = 0x08,
	LastAccumulate = 0x09,
}

impl StorageKey {
	/// Service storage key for an unkeyed item.
	pub const fn key(self) -> [u8; 1] {
		[self as u8]
	}

	/// Service storage key for a parachain entry: `[tag] || SCALE(para_id)`.
	pub fn para_key(self, para_id: ParaId) -> [u8; 5] {
		let [key_prefix] = self.key();
		let [b0, b1, b2, b3] = u32::from(para_id).to_le_bytes();
		[key_prefix, b0, b1, b2, b3]
	}
}

/// Per-parachain metadata (spec §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ParaInfo {
	/// Current head data (output of last included block).
	pub head_data: HeadData,
	/// Currently active validation code, or `None` for a freshly-registered
	/// parachain. Spec §6.
	pub validation_code: Option<ValidationCodeRef>,
	/// Code announced for upgrade, awaiting an `Apply`; `None` when no
	/// announcement stands. An announcement carries no deadline and stays
	/// standing until applied or superseded. Spec §5.2.
	pub announced_upgrade: Option<ValidationCodeRef>,
	/// Total state balance allocated to this parachain. Set exclusively by the
	/// Coretime chain via `parachain_set_state_balance`. Spec §6.1.
	#[codec(compact)]
	pub total_state_balance: Balance,
	/// State balance currently consumed by this parachain's footprint. Spec §6.1.
	#[codec(compact)]
	pub used_state_balance: Balance,
	/// Set once `parachain_clean_up` has begun deregistering this parachain but
	/// some preimage still awaits its second, expunging `forget`. Spec §6.4.
	pub is_deregistering: bool,
}

impl ParaInfo {
	/// Whether increasing the used balance stays within the allocated balance.
	pub fn has_headroom(&self, delta: Balance) -> bool {
		self.used_state_balance.saturating_add(delta) <= self.total_state_balance
	}

	/// Grow the used balance after checking headroom.
	pub fn charge(&mut self, delta: Balance) {
		self.used_state_balance = self.used_state_balance.saturating_add(delta);
	}

	/// Shrink the used balance after releasing state.
	pub fn refund(&mut self, delta: Balance) {
		self.used_state_balance = self.used_state_balance.saturating_sub(delta);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::{
		validation_code_hash_bytes, HeadData, ParaId, ValidationCodeHash, ValidationCodeRef,
	};
	use alloc::vec;
	use codec::{Decode, Encode};

	#[test]
	fn storage_keys_works() {
		assert_eq!(StorageKey::LastAccumulate.key(), [0x09]);
		assert_eq!(StorageKey::Parachains.para_key(ParaId::from(0)), [0; 5]);
		assert_eq!(
			StorageKey::ParachainLog.para_key(ParaId::from(0x1234_5678)),
			[0x01, 0x78, 0x56, 0x34, 0x12]
		);
	}

	/// Both optional fields are `Some` so a degenerate all-`None` implementation cannot pass.
	///
	/// Byte layout (SCALE encoding, field-by-field):
	///   head_data              `08 ca fe`         compact(2) prefix + bytes
	///   validation_code Some   `01`
	///   code_ref.hash          `[0x11; 32]`       `ValidationCodeHash` inner `[u8; 32]`
	///   code_ref.len           `01 00 00 00`      `1u32` LE
	///   announced_upgrade Some `01`
	///   code_ref.hash          `[0x22; 32]`
	///   code_ref.len           `02 00 00 00`      `2u32` LE
	///   total_state_balance    `28`               compact(10) = 10 << 2
	///   used_state_balance     `14`               compact(5)  =  5 << 2
	///   is_deregistering       `01`               true
	///
	/// 80 bytes total: the `ValidationCode` wrapper and its `pinned` bit, and the
	/// `(ValidationCode, Timeslot)` tuple, are gone — each optional is now a bare
	/// `ValidationCodeRef` (32-byte hash + 4-byte LE length).
	///
	/// To regenerate: SCALE-encode an equivalent `service::state::para_info::ParaInfo`
	/// (same field values) with `codec::Encode::encode()` and print the bytes as hex.
	#[test]
	fn decodes_head_data_from_wire_bytes() {
		let bytes: &[u8] = &[
			0x08, 0xca, 0xfe, 0x01, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
			0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
			0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x01, 0x00, 0x00, 0x00, 0x01, 0x22,
			0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
			0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
			0x22, 0x22, 0x22, 0x02, 0x00, 0x00, 0x00, 0x28, 0x14, 0x01,
		];
		let info = ParaInfo::decode(&mut &bytes[..]).expect("well-formed fixture; qed");
		assert_eq!(info.encode(), bytes);
		let expected_head = HeadData::try_from(vec![0xca, 0xfe]).expect("2 bytes < 4 KiB; qed");
		assert_eq!(info.head_data, expected_head);
		let vc = info.validation_code.expect("Some in fixture; qed");
		assert_eq!(validation_code_hash_bytes(&vc.hash), [0x11u8; 32]);
		assert_eq!(vc.len, 1);
		let announced = info.announced_upgrade.expect("Some in fixture; qed");
		assert_eq!(validation_code_hash_bytes(&announced.hash), [0x22u8; 32]);
		assert_eq!(announced.len, 2);
		assert_eq!(info.total_state_balance, 10);
		assert_eq!(info.used_state_balance, 5);
		assert!(info.is_deregistering);
	}

	/// Encodes then decodes with both `Some` variants to prove `Encode`/`Decode` are consistent.
	#[test]
	fn round_trips_with_both_some_variants() {
		let code_ref_a = ValidationCodeRef { hash: ValidationCodeHash::from([0xab; 32]), len: 512 };
		let code_ref_b =
			ValidationCodeRef { hash: ValidationCodeHash::from([0xcd; 32]), len: 1024 };
		let head_data: HeadData =
			HeadData::try_from(vec![0xde, 0xad, 0xbe, 0xef]).expect("4 bytes < 4 KiB; qed");
		let original = ParaInfo {
			head_data,
			validation_code: Some(code_ref_a),
			announced_upgrade: Some(code_ref_b),
			total_state_balance: 100_000,
			used_state_balance: 42_000,
			is_deregistering: false,
		};
		let encoded = original.encode();
		let decoded = ParaInfo::decode(&mut &encoded[..]).expect("own encoding decodes; qed");
		assert_eq!(original, decoded);
	}
}
