//! Mirror of `service/src/state/para_info.rs`, `service/src/state/mod.rs` (`Tag` +
//! `storage_key`), and `cumulus/src/lib.rs` (`service_state::para_info_key`).
//!
//! IMPORTANT: this file MUST be kept in step with the canonical definitions. Field
//! order, field types, and `#[codec(...)]` attributes are **wire format**: reordering
//! fields or dropping a `compact` silently breaks decoding of live service state.
//! When the canonical changes, update this file and re-run the byte-pinning tests.
//!
//! Sources mirrored (read-only; do not edit those files):
//!   - `service/src/state/para_info.rs`   — `ParaInfo`
//!   - `service/src/state/mod.rs`         — `Tag`, `storage_key`
//!   - `cumulus/src/lib.rs service_state` — `para_info_key`

use crate::types::{Balance, HeadData, ParaId, ValidationCodeRef};
use alloc::vec::Vec;
use codec::{Decode, Encode};

/// Storage-item tags (spec §3.1, "Storage key encoding").
///
/// Mirrors `service::state::Tag`.
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
	IncomingTransferBuckets = 0x07,
	KeyValueStorage = 0x08,
}

/// The full JAM storage key for a map entry: `[tag] || SCALE(key)`.
///
/// Mirrors `service::state::storage_key`.
pub fn storage_key(tag: Tag, key: &impl Encode) -> Vec<u8> {
	let mut k = Vec::with_capacity(1 + key.encoded_size());
	k.push(tag as u8);
	key.encode_to(&mut k);
	k
}

/// Storage key of the para's [`ParaInfo`] entry in the parachain service.
///
/// Byte-identical to `cumulus::service_state::para_info_key`.
pub fn para_info_key(para_id: ParaId) -> Vec<u8> {
	storage_key(Tag::Parachains, &para_id)
}

/// Per-parachain metadata (spec §3.1).
///
/// Mirrors `service::state::para_info::ParaInfo`. Field order, types, and codec
/// attributes match the canonical definition exactly; any drift is a silent
/// wire-format break.
///
/// The write-side methods (`has_headroom`, `charge`, `refund`) are deliberately
/// omitted: a PVM guest only reads this type and has no use for them.
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::types::{HeadData, ParaId, ValidationCodeHash, ValidationCodeRef};
	use alloc::vec;
	use codec::{Decode, Encode};

	/// Key must be byte-identical to `cumulus::service_state::para_info_key`.
	/// Pinned from `cumulus/src/lib.rs service_state::tests`.
	#[test]
	fn para_info_key_is_tag_then_scale_para_id() {
		// Canonical pinning case from cumulus/src/lib.rs.
		assert_eq!(para_info_key(ParaId::from(3)), vec![0x00, 3, 0, 0, 0]);
		// Para 0: all-zeroes case.
		assert_eq!(para_info_key(ParaId::from(0)), vec![0x00, 0, 0, 0, 0]);
		// Para 2000 (0x000007D0): pins the little-endian layout beyond a single byte.
		assert_eq!(para_info_key(ParaId::from(2000)), vec![0x00, 0xd0, 0x07, 0, 0]);
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
		let expected_head = HeadData::try_from(vec![0xca, 0xfe]).expect("2 bytes < 4 KiB; qed");
		assert_eq!(info.head_data, expected_head);
		let vc = info.validation_code.expect("Some in fixture; qed");
		assert_eq!(vc.hash.0, [0x11u8; 32]);
		assert_eq!(vc.len, 1);
		let announced = info.announced_upgrade.expect("Some in fixture; qed");
		assert_eq!(announced.hash.0, [0x22u8; 32]);
		assert_eq!(announced.len, 2);
		assert_eq!(info.total_state_balance, 10);
		assert_eq!(info.used_state_balance, 5);
		assert!(info.is_deregistering);
	}

	/// Encodes then decodes with both `Some` variants to prove `Encode`/`Decode` are consistent.
	#[test]
	fn round_trips_with_both_some_variants() {
		let code_ref_a = ValidationCodeRef { hash: ValidationCodeHash([0xab; 32]), len: 512 };
		let code_ref_b = ValidationCodeRef { hash: ValidationCodeHash([0xcd; 32]), len: 1024 };
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
