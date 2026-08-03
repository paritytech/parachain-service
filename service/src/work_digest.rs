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
    Opaque(Vec<u8>),
    // FIXME
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
}
