//! Refine output of the parachain service.

use bounded_collections::{BoundedVec, ConstU32};
use codec::{Decode, Encode};
use jam_types::Hash;
use parachain_service_interface::upward_message::UpwardMessages;

// Shared wire types live in the interface crate; re-exported here since they are
// part of the digest's shape.
pub use parachain_service_interface::types::{
	HeadData, ParaId, Timeslot, ValidationCodeHash, ValidationCodeRef,
};

#[cfg(feature = "std")]
use jam_std_common::hash_raw;

/// Maximum combined encoded size of all `ParachainWorkDigest`s and the auth
/// trace — the Gray Paper's `W_R` (`C_maxreportvarsize`).
pub const MAX_REFINE_OUTPUT_SIZE: usize = 48 * 1024;

/// The parachain service's Refine output for one parachain candidate. Side
/// effects from host functions are carried in `upward_messages` and applied by
/// Accumulate. Spec §3.3.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ParachainWorkDigest {
	/// PVF validation succeeded.
	Ok {
		/// The parachain this digest belongs to.
		para_id: ParaId,
		/// The validation code that Refine actually used to check the candidate.
		validation_code: ValidationCodeRef,
		/// Hash of the parent head data this candidate was built on top of.
		parent_head_hash: Hash,
		/// New head data produced by the parachain block.
		head_data: HeadData,
		/// Upward messages emitted through host functions during Refine.
		upward_messages: UpwardMessages,
		/// The work package's lookup-anchor timeslot.
		lookup_anchor: Timeslot,
	},
	/// PVF execution failed (e.g. invalid PoV, bad state proof, panic).
	Err {
		/// The parachain this failure belongs to.
		para_id: ParaId,
		/// Structured failure reason.
		error: RefineLog,
	},
}

impl ParachainWorkDigest {
	pub fn para_id(&self) -> ParaId {
		match self {
			Self::Ok { para_id, .. } => *para_id,
			Self::Err { para_id, .. } => *para_id,
		}
	}
}

/// Structured reason a `refine` invocation failed.
///
/// Every variant is raised only after §4.1 step 2 fixes an authoritative
/// `para_id`, since the entry lands in `parachain_log[para_id]`. Failures before
/// that panic instead (§4.2).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum RefineLog {
	/// `historical_lookup(validation_code_hash)` returned `None`: the
	/// validation code preimage is not available in the service's store
	/// at the lookup-anchor. See §4.1 step 3.
	InvalidCodeHash,
	/// Opaque payload supplied by the PVF via `report_error(data)` before
	/// failing the execution (max 1024 bytes).
	Opaque(BoundedVec<u8, ConstU32<1024>>),
	/// A `set_validator_keys` chunk contained more than 30 keys, or
	/// `set_validator_keys` was called more than once in a single Refine
	/// invocation. See §4.3, §5.3.
	SetValidatorKeysTooManyKeys,
	/// The PVF emitted more than 1024 upward messages in a single Refine
	/// invocation. See §4.3.
	TooManyUpwardMessages,
	/// The PVF invoked a host function restricted to another parachain
	/// (Asset Hub or the Coretime chain). See §4.3.
	RestrictedHostFunction,
	/// An `assign_core` queue outside 1..=`AUTHORIZER_QUEUE_LEN` hashes — and any
	/// queue other than exactly `AUTHORIZER_QUEUE_LEN` when handing the core to a
	/// new assigner. See §4.3.
	InvalidAuthorizerQueue,
	/// The work package payload failed to be decoded.
	MalformedPayload,
	/// The encoded `ParachainWorkDigest` (head data + upward messages) would
	/// exceed the Gray Paper's 48 KiB combined result-blob + auth-trace
	/// budget. See §4.1.
	RefineOutputTooLarge,
	/// The PVF exited without calling `set_parent_head_hash` and/or `set_head`
	/// exactly once. Both head declarations are mandatory. See §4.2.
	MissingHeadDeclaration,
}

/// The maximum byte length of a `report_error` payload. Spec §4.3.
pub const MAX_REPORT_ERROR_PAYLOAD: u32 = 1024;

#[cfg(feature = "test-utils")]
impl ParachainWorkDigest {
	pub fn try_into_log(self) -> Result<RefineLog, ()> {
		match self {
			Self::Ok { .. } => Err(()),
			Self::Err { error, .. } => Ok(error),
		}
	}
}

/// Hash a validation-code blob into its [`ValidationCodeHash`].
#[cfg(feature = "std")]
pub fn validation_code_hash(code: &[u8]) -> ValidationCodeHash {
	ValidationCodeHash(hash_raw(code))
}
