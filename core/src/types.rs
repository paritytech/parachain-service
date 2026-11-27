//! Types shared by the service, authorizers, and collator tooling.

use bounded_collections::{BoundedVec, ConstU32};
use codec::{Decode, Encode};

/// A JAM timeslot.
pub use jam_types::Slot as Timeslot;

/// A JAM core index.
pub use jam_types::CoreIndex;

/// A JAM service identifier.
pub use jam_types::ServiceId;

/// Monotonic key for an `incoming_transfers` bucket (spec §3.1).
pub type BucketId = u64;

/// A JAM balance.
pub use jam_types::Balance;

pub use jam_types::{AuthorizerHash, Memo, OpaqueValKeyset};

/// Maximum size of a parachain's head data (4 KiB, spec §3.1).
pub const MAX_HEAD_DATA_SIZE: u32 = 4 * 1024;

/// Head data produced by a parachain block (spec §3.1).
pub type HeadData = BoundedVec<u8, ConstU32<MAX_HEAD_DATA_SIZE>>;

pub use polkadot_parachain_primitives::primitives::Id as ParaId;

/// Coretime chain `ParaId`. Owns parachain management (§6) and core assignment (§7).
pub const CORETIME_PARA_ID: ParaId = ParaId::new(1005);

/// Asset Hub `ParaId`. Owns validator-key updates (§5.3), service upgrades (§5.4),
/// and transfers (§5.1).
pub const ASSET_HUB_PARA_ID: ParaId = ParaId::new(1000);

/// Hash of a parachain's validation code (PVF) preimage.
pub use polkadot_parachain_primitives::primitives::ValidationCodeHash;

/// Validation code reference keyed by `(hash, len)` in the preimage registry (spec §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ValidationCodeRef {
	pub hash: ValidationCodeHash,
	pub len: u32,
}

impl ValidationCodeRef {
	/// Returns whether `(hash, len)` matches this validation code.
	pub fn is(&self, hash: &ValidationCodeHash, len: u32) -> bool {
		self.hash == *hash && self.len == len
	}
}
