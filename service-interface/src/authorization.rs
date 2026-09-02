//! The authorization wire types the `service` and `authorizer` programs both speak.
//!
//! They live here rather than in the authorizer crate because the service must decode an
//! [`AuthTrace`] without linking the authorizer: the two are separate blobs, and depending on the
//! authorizer crate drags its `is_authorized` entry point (and a curve implementation) into the
//! service's own.
//! The token and the config stay with the authorizer, which is the only program that reads them.
//! [`Command`] is the service's own: since 6a.4 a command travels in a work item's payload, which
//! the authorizer never looks at.

use crate::types::{AuthorizerHash, CoreIndex, ParaId};
use codec::{Decode, Encode, MaxEncodedLen};

/// A collator public key: the raw 32 bytes, whichever curve they are on.
///
/// Both aura schemes have the same shape here, which is what lets everything but the verifier
/// itself stay scheme-blind. Which curve a core's collators sign on is settled by the authorizer
/// blob its queue commits to.
pub type CollatorKey = [u8; 32];

/// A collator's signature over an authorization token's signing payload, raw 64 bytes.
pub type CollatorSignature = [u8; 64];

/// A core-assignment command, carried as a work item's payload behind
/// [`CONTROL_COMMAND_PREFIX`].
///
/// The payload is the channel because the Coretime chain the real service takes its assignments
/// from does not exist on a test network, and because Accumulate — where `assign` has to be
/// called — sees neither the work package nor its authorization. Refine reads the command out of
/// the payload and forwards it in its work output.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub enum Command {
	/// Fill `core`'s authorizer queue with `authorizer`.
	///
	/// The hash is carried rather than derived because Accumulate cannot see an authorizer
	/// config — not even its own package's — so it has no way to build one for `para_id`.
	/// `para_id` is therefore advisory: it is what the service logs, not what it writes.
	Assign { para_id: ParaId, core: CoreIndex, authorizer: AuthorizerHash },
	/// Park `core`: fill its queue with `parked_authorizer`, which is the same AURA authorizer
	/// code under a config naming no para at all.
	///
	/// Assignment to this service is one-way. A parked core keeps the authorizer that lets
	/// commands reach it and refuses parachain work, because no para is assigned to it — rather
	/// than going back to the null authorizer, which would leave it deaf to the very command that
	/// would bring it back. The hash travels for the same reason `Assign`'s does, and it cannot
	/// be derived from the carrier's own config either: the core being parked may be some other
	/// core, under some other collator set.
	Free { core: CoreIndex, parked_authorizer: AuthorizerHash },
}

/// The work-item payload prefix that marks the rest of the payload as a SCALE-encoded
/// [`Command`].
///
/// A command needs a work item of its own because the Gray Paper admits no package without one,
/// and it needs a marker because refine has to tell a command from a parachain block without
/// guessing at a decode.
pub const CONTROL_COMMAND_PREFIX: &[u8] = b"parasim:control";

/// What Is-Authorized hands to Refine and Accumulate for every work item in the package.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub struct AuthTrace {
	pub author_key: CollatorKey,
	/// Whether the package was admitted through the authorizer's `sudo` lane.
	///
	/// Refine takes a command out of a payload only when this is set, so a command cannot be
	/// smuggled in on the ordinary collator lane: the trace is the only thing refine gets to see
	/// of the authorization, and only the authorizer can set it.
	pub sudo: bool,
}
