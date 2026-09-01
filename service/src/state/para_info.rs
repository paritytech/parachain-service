//! Per-parachain record (spec §3.1) and its storage accessors.

use crate::state::{self, Tag};
use codec::{Compact, Decode, Encode};
use parachain_service_interface::types::{Balance, HeadData, ParaId, Timeslot, ValidationCodeRef};

/// A validation code with its reference and `pinned` flag, recording whether the
/// parachain has *also* solicited it itself, on top of the service's own
/// code-upgrade solicit. Spec §5.2.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ValidationCode {
	pub code_ref: ValidationCodeRef,
	pub pinned: bool,
}

/// Per-parachain metadata (spec §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ParaInfo {
	/// Current head data (output of last included block).
	pub head_data: HeadData,
	/// Currently active validation code, or `None` for a freshly-registered
	/// parachain. Spec §6.
	pub validation_code: Option<ValidationCode>,
	/// Pending code upgrade, if any: the new validation code and the deadline
	/// timeslot after which the upgrade is rejected. Spec §5.2.
	pub pending_upgrade: Option<(ValidationCode, Timeslot)>,
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
	/// §6.1 write-time invariant: `true` iff growing `used_state_balance` by
	/// `delta` stays within `total_state_balance`.
	pub fn has_headroom(&self, delta: Balance) -> bool {
		self.used_state_balance.saturating_add(delta) <= self.total_state_balance
	}

	/// Grow `used_state_balance`; the caller must have checked headroom.
	pub fn charge(&mut self, delta: Balance) {
		self.used_state_balance = self.used_state_balance.saturating_add(delta);
	}

	/// Shrink `used_state_balance` (a release/refund).
	pub fn refund(&mut self, delta: Balance) {
		self.used_state_balance = self.used_state_balance.saturating_sub(delta);
	}
}

// Compile-time check that `Compact<Balance>` derives from `u64` (DECISIONS.md D-3).
const _: fn(Balance) -> Compact<u64> = Compact::<u64>;

/// Storage accessors for the `parachains` map (tag `0x00`).
pub struct Parachains;

impl Parachains {
	pub fn get(para_id: ParaId) -> Option<ParaInfo> {
		state::read(Tag::Parachains, &para_id)
	}

	pub fn set(para_id: ParaId, info: &ParaInfo) {
		state::write(Tag::Parachains, &para_id, info)
	}

	pub fn remove(para_id: ParaId) {
		state::clear(Tag::Parachains, &para_id)
	}

	/// §5.1 step 1: a para is live iff registered.
	pub fn is_live(para_id: ParaId) -> bool {
		Self::get(para_id).is_some()
	}
}
