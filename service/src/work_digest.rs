//! Refine output of the parachain service.

use alloc::vec::Vec;

use codec::{Decode, Encode};
use jam_types::Hash;
use parachain_support::types::ParaId;

/// A JAM timeslot.
pub type Timeslot = u32;

/// Hash of a parachain's validation code (PVF) preimage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ValidationCodeHash(pub Hash); // TODO maybe use own hash type

/// A validation code reference: its hash plus its byte length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
pub struct ValidationCodeRef {
    pub hash: ValidationCodeHash,
    pub len: u32,
}

/// New head data produced by a parachain block.
pub type HeadData = Vec<u8>;

/// Upward messages emitted via host functions during Refine and replayed in order by Accumulate.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum UpwardMessage {}

/// Structured reason a `refine` invocation failed.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum RefineLog {
    /// `historical_lookup(validation_code_hash)` returned `None`: the
    /// validation code preimage is not available in the service's store
    /// at the lookup-anchor. See §4.1 step 3.
    InvalidCodeHash,
    /// Opaque payload supplied by the PVF via `report_error(data)` before
    /// failing the execution (max 1024 bytes).
    //Opaque(BoundedVec<u8, 1024>),
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
    /// The opaque `AuthorizerConfig` blob failed to decode. See §4.1 step 1.
    MalformedAuthorizerConfig,
    /// The encoded `ParachainWorkDigest` (head data + upward messages) would
    /// exceed the Gray Paper's 48 KiB combined result-blob + auth-trace
    /// budget. See §4.1.
    WorkDigestTooLarge,
    /// The PVF exited without calling `set_parent_head_hash` and/or `set_head`
    /// exactly once. Both head declarations are mandatory. See §4.2.
    MissingHeadDeclaration,
    /// Not exactly two extrinsics per Work Item.
    InvalidExtrinsicCount,
}

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
        upward_messages: Vec<UpwardMessage>,
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
    /// Could not determine for which ParaID a Work Item was intended.
    // FIXME: Update spec with this
    AuthError { error: RefineLog },
}

//#[cfg(feature = "std")]
impl ParachainWorkDigest {
    pub fn try_into_log(self) -> Result<RefineLog, ()> {
        match self {
            Self::Ok { .. } => Err(()),
            Self::Err { error, .. } => Ok(error),
            Self::AuthError { error, .. } => Ok(error),
        }
    }
}
