//! Types shared by the service, authorizers, and collator tooling.
//!
//! NOTE: jam-types wrapper structs (`jam_types::AuthorizerHash`, `jam_types::Memo`,
//! `jam_types::OpaqueValKeyset`) implement `jam_codec`'s traits, not the SCALE `codec`
//! ones these wire types derive. Fields therefore use plain byte arrays and are
//! converted into the jam-types wrappers at the JAM host-call sites.

extern crate alloc;

use bounded_collections::{BoundedVec, ConstU32};
use codec::{Decode, Encode};

/// A JAM timeslot (`jam_types::Slot`).
pub type Timeslot = u32;
/// A 32-byte hash, layout-compatible with `jam_types::Hash`.
pub type Hash = [u8; 32];
/// A JAM service identifier (`jam_types::ServiceId`).
pub type ServiceId = u32;
/// A JAM core index (`jam_types::CoreIndex`).
pub type CoreIndex = u16;
/// Key of one `incoming_transfers` bucket: a `u64` the service allocates by
/// incrementing, deliberately unrelated to the arrival timeslot (spec §3.1).
pub type BucketId = u64;
/// A JAM balance. The design doc says `Compact<u128>`, but JAM's `Balance` is `u64`
/// (see DECISIONS.md D-3); wire encodings use `Compact<u64>`.
pub type Balance = u64;
/// An authorizer hash (`H(code_hash ⌢ config)`), layout-compatible with
/// `jam_types::AuthorizerHash`.
pub type AuthorizerHash = [u8; 32];
/// Fixed 128-byte transfer memo, matching Gray Paper `C_memosize = 128` and
/// layout-compatible with `jam_types::Memo`.
pub type Memo = [u8; 128];
/// Opaque 336-byte validator key blob, layout-compatible with
/// `jam_types::OpaqueValKeyset` (bandersnatch 32 + ed25519 32 + bls 144 + metadata 128).
pub type ValidatorKey = [u8; VALIDATOR_KEY_SIZE];

/// Byte size of one [`ValidatorKey`]. Spec §5.3.
pub const VALIDATOR_KEY_SIZE: usize = 336;

/// Head data is capped at 4 KiB to bound the per-parachain footprint that
/// `ParaInfo` contributes to the baseline state-balance reservation. Spec §3.1, §6.1.
pub const MAX_HEAD_DATA_SIZE: u32 = 4 * 1024;

/// New head data produced by a parachain block. Spec §3.1.
pub type HeadData = BoundedVec<u8, ConstU32<MAX_HEAD_DATA_SIZE>>;

pub use polkadot_parachain_primitives::primitives::Id as ParaId;

/// The Coretime chain's `ParaId`. Owns parachain management (§6) and core
/// assignment (§7).
///
/// Compile-time constant per DECISIONS.md D-2; matches the Quint model's
/// `CoretimeParaId`. FIXME: production needs a bootstrap/governance story for
/// migrating this identity.
pub const CORETIME_PARA_ID: ParaId = ParaId::new(1);

/// Asset Hub's `ParaId`. Owns validator-key updates (§5.3), service self-upgrade
/// (§5.4), outbound transfers, and the incoming-transfer queue (§5.1).
///
/// Compile-time constant per DECISIONS.md D-2; matches the Quint model's
/// `AssetHubParaId`. FIXME: see [`CORETIME_PARA_ID`].
pub const ASSET_HUB_PARA_ID: ParaId = ParaId::new(2);

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
	pub fn is(&self, hash: &Hash, len: u32) -> bool {
		validation_code_hash_bytes(&self.hash) == *hash && self.len == len
	}
}
