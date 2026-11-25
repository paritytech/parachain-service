//! Types shared by the service, authorizers, and collator tooling.
//!
//! `jam-types` wrappers (`AuthorizerHash`, `Memo`, `OpaqueValKeyset`) use `jam_codec` rather
//! than SCALE `codec`, so wire types use raw byte arrays and convert at JAM host-call sites.

use bounded_collections::{BoundedVec, ConstU32};
use codec::{Decode, Encode};

/// A JAM timeslot.
pub use jam_types::Slot as Timeslot;

/// A 32-byte hash.
pub type Hash = [u8; 32];

/// A JAM core index.
pub use jam_types::CoreIndex;

/// A JAM service identifier.
pub use jam_types::ServiceId;

/// Monotonic key for an `incoming_transfers` bucket (spec §3.1).
pub type BucketId = u64;

/// A JAM balance.
pub use jam_types::Balance;

/// An authorizer hash (`H(code_hash ⌢ config)`).
pub type AuthorizerHash = [u8; 32];

/// Fixed 128-byte transfer memo (`C_memosize = 128`).
pub type Memo = [u8; 128];

/// Opaque 336-byte validator key blob (`jam_types::OpaqueValKeyset`).
pub type ValidatorKey = [u8; VALIDATOR_KEY_SIZE];

/// Byte size of one [`ValidatorKey`] (spec §5.3).
pub const VALIDATOR_KEY_SIZE: usize = 336;

/// Maximum size of a parachain's head data (4 KiB, spec §3.1).
pub const MAX_HEAD_DATA_SIZE: u32 = 4 * 1024;

/// Head data produced by a parachain block (spec §3.1).
pub type HeadData = BoundedVec<u8, ConstU32<MAX_HEAD_DATA_SIZE>>;

pub use polkadot_parachain_primitives::primitives::Id as ParaId;

/// Coretime chain `ParaId`. Owns parachain management (§6) and core assignment (§7).
///
/// FIXME: production needs a bootstrap/governance story for migrating this identity.
pub const CORETIME_PARA_ID: ParaId = ParaId::new(1);

/// Asset Hub `ParaId`. Owns validator-key updates (§5.3), service upgrades (§5.4),
/// and transfers (§5.1).
///
/// FIXME: see [`CORETIME_PARA_ID`].
pub const ASSET_HUB_PARA_ID: ParaId = ParaId::new(2);

/// Hash of a parachain's validation code (PVF) preimage.
pub use polkadot_parachain_primitives::primitives::ValidationCodeHash;

/// Convert a [`ValidationCodeHash`] to raw bytes.
pub fn validation_code_hash_bytes(hash: &ValidationCodeHash) -> Hash {
	hash.as_ref().try_into().expect("validation code hashes are 32 bytes")
}

/// Validation code reference keyed by `(hash, len)` in the preimage registry (spec §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ValidationCodeRef {
	pub hash: ValidationCodeHash,
	pub len: u32,
}

impl ValidationCodeRef {
	/// Returns whether `(hash, len)` matches this validation code.
	pub fn is(&self, hash: &Hash, len: u32) -> bool {
		validation_code_hash_bytes(&self.hash) == *hash && self.len == len
	}
}
