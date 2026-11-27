//! Types shared by the service, authorizers, and collator tooling.
//!
//! JAM wrapper types use `jam_codec`; the explicitly named `Wire` types retain
//! the raw fixed-width SCALE representation used by the PoC ABI and storage.

extern crate alloc;

use bounded_collections::{BoundedVec, ConstU32};
use codec::{Compact, Decode, Encode};
use jam_types::Hash;

/// A JAM timeslot (`jam_types::Slot`).
pub use jam_types::Slot as Timeslot;
/// A JAM core index (`jam_types::CoreIndex`).
pub use jam_types::CoreIndex;
/// A JAM service identifier (`jam_types::ServiceId`).
pub use jam_types::ServiceId;
/// Key of one `incoming_transfers` bucket: a `u64` the service allocates by
/// incrementing, deliberately unrelated to the arrival timeslot (spec §3.1).
pub type BucketId = u64;
/// A JAM balance. The design doc says `Compact<u128>`, but JAM's `Balance` is `u64`
/// (see DECISIONS.md D-3); wire encodings use `Compact<u64>`.
pub use jam_types::Balance;
pub use jam_types::{AuthorizerHash, Memo, OpaqueValKeyset};

/// SCALE representation of an authorizer hash at the PoC wire boundary.
pub type WireAuthorizerHash = [u8; 32];
/// SCALE representation of a fixed-size JAM transfer memo.
pub type WireMemo = [u8; 128];
/// SCALE representation of a JAM validator keyset.
pub type WireValidatorKey = [u8; VALIDATOR_KEY_SIZE];

/// Byte size of one [`WireValidatorKey`]. Spec §5.3.
pub const VALIDATOR_KEY_SIZE: usize = 336;

/// Caller-supplied identifier of an upward message, logged with every
/// `AccumulateLog` the message causes. Spec §4.3.
pub type MessageId = Compact<u64>;

/// Head data is capped at 4 KiB to bound the per-parachain footprint that
/// `ParaInfo` contributes to the baseline state-balance reservation. Spec §3.1, §6.1.
pub const MAX_HEAD_DATA_SIZE: u32 = 4 * 1024;

/// New head data produced by a parachain block. Spec §3.1.
pub type HeadData = BoundedVec<u8, ConstU32<MAX_HEAD_DATA_SIZE>>;

pub use polkadot_parachain_primitives::primitives::Id as ParaId;

/// The Coretime chain's `ParaId`. Owns parachain management (§6) and core
/// assignment (§7).
///
/// Production Coretime identity.
pub const CORETIME_PARA_ID: ParaId = ParaId::new(1005);

/// Asset Hub's `ParaId`. Owns validator-key updates (§5.3), service self-upgrade
/// (§5.4), outbound transfers, and the incoming-transfer queue (§5.1).
///
/// Production Asset Hub identity.
pub const ASSET_HUB_PARA_ID: ParaId = ParaId::new(1000);

/// Hash of a parachain's validation code (PVF) preimage.
pub use polkadot_parachain_primitives::primitives::ValidationCodeHash;

/// Convert an SDK validation-code hash to the raw bytes used by JAM APIs.
pub fn validation_code_hash_bytes(hash: &ValidationCodeHash) -> Hash {
	hash.as_ref().try_into().expect("validation code hashes are 32 bytes")
}

/// A validation code reference: its hash plus its byte length.
///
/// Both components identify the preimage: the registry is keyed by `(hash, len)`,
/// so the same hash at a different length is a distinct preimage (§6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ValidationCodeRef {
	pub hash: ValidationCodeHash,
	pub len: u32,
}

impl ValidationCodeRef {
	/// Does the `(hash, len)` pair of a `solicit`/`forget` name this validation code?
	pub fn is(&self, hash: &ValidationCodeHash, len: u32) -> bool {
		self.hash == *hash && self.len == len
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn validation_code_reference_matches_hash_and_length() {
		let hash = ValidationCodeHash::from([1; 32]);
		let code = ValidationCodeRef { hash, len: 42 };
		assert!(code.is(&hash, 42));
		assert!(!code.is(&hash, 43));
		assert!(!code.is(&ValidationCodeHash::from([2; 32]), 42));
	}
}
