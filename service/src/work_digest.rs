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
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum RefineLog {
	/// `historical_lookup(validation_code_hash)` returned `None`: the
	/// validation code preimage is not available in the service's store
	/// at the lookup-anchor. See §4.1 step 3.
	InvalidCodeHash,
	/// The PVF could not be parsed as a PVM program, its `jam_validate_block` entry
	/// point was not found, or the inner PVM could not be instantiated from it.
	InvalidCode,
	/// The PVF ran but did not validate the candidate: it panicked, trapped, ran out of
	/// gas, or otherwise failed to return head data.
	// FIXME: split into distinct causes (panic / OOG / stray host call) once needed.
	ValidationFailed,
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
	/// The authorizer config's `authorized_paras` prefix length does not
	/// match the work package's item count. See §4.1 step 1.
	AuthConfigMismatch,
	/// The work package has 0 items or more than 1 item; only single-item
	/// packages are currently supported (§3.2).
	InvalidItemCount,
	/// The work package payload failed to be decoded.
	MalformedPayload,
	/// The encoded `ParachainWorkDigest` (head data + upward messages) would
	/// exceed the Gray Paper's 48 KiB combined result-blob + auth-trace
	/// budget. See §4.1.
	RefineOutputTooLarge,
	/// The PVF exited without calling `set_parent_head_hash` and/or `set_head`
	/// exactly once. Both head declarations are mandatory. See §4.2.
	MissingHeadDeclaration,
	/// The PVF called `set_head` with more than 4 KiB of head data (§3.1).
	// TODO: not in the spec's RefineLog — §4.2/§3.1 leave an oversized `set_head`
	// unspecified. Needs upstreaming.
	HeadDataTooLarge,
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
