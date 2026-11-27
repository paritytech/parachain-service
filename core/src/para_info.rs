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

	/// Active validation code, or `None` if newly registered (spec §6).
	pub validation_code: Option<ValidationCodeRef>,

	/// Announced code upgrade awaiting `Apply` (spec §5.2).
	pub announced_upgrade: Option<ValidationCodeRef>,

	/// Total state balance allocated by Coretime (spec §6.1).
	#[codec(compact)]
	pub total_state_balance: Balance,

	/// State balance currently consumed by this parachain (spec §6.1).
	#[codec(compact)]
	pub used_state_balance: Balance,

	/// Set while deregistration awaits final preimage cleanup (spec §6.4).
	pub is_deregistering: bool,
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::{HeadData, ParaId, ValidationCodeHash, ValidationCodeRef};
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

	#[test]
	fn wire_decode_works() {
		let bytes: &[u8] = &[
			0x08, 0xca, 0xfe, 0x01, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
			0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
			0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x01, 0x00, 0x00, 0x00, 0x01, 0x22,
			0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
			0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
			0x22, 0x22, 0x22, 0x02, 0x00, 0x00, 0x00, 0x28, 0x14, 0x01,
		];

		let info = ParaInfo::decode(&mut &bytes[..]).unwrap();
		assert_eq!(info.encode(), bytes);

		let expected_head = HeadData::try_from(vec![0xca, 0xfe]).unwrap();
		assert_eq!(info.head_data, expected_head);

		let vc = info.validation_code.unwrap();
		assert_eq!(vc.hash, ValidationCodeHash::from([0x11; 32]));
		assert_eq!(vc.len, 1);

		let announced = info.announced_upgrade.unwrap();
		assert_eq!(announced.hash, ValidationCodeHash::from([0x22; 32]));
		assert_eq!(announced.len, 2);

		assert_eq!(info.total_state_balance, 10);
		assert_eq!(info.used_state_balance, 5);
		assert!(info.is_deregistering);
	}

	#[test]
	fn round_trip_works() {
		let code_ref_a = ValidationCodeRef { hash: ValidationCodeHash::from([0xab; 32]), len: 512 };
		let code_ref_b =
			ValidationCodeRef { hash: ValidationCodeHash::from([0xcd; 32]), len: 1024 };
		let head_data = HeadData::try_from(vec![0xde, 0xad, 0xbe, 0xef]).unwrap();

		let original = ParaInfo {
			head_data,
			validation_code: Some(code_ref_a),
			announced_upgrade: Some(code_ref_b),
			total_state_balance: 100_000,
			used_state_balance: 42_000,
			is_deregistering: false,
		};

		let encoded = original.encode();
		let decoded = ParaInfo::decode(&mut &encoded[..]).unwrap();
		assert_eq!(original, decoded);
	}
}
