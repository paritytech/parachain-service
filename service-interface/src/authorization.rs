//! The authorization wire types the `service` and `authorizer` programs both speak.
//!
//! They live here rather than in the authorizer crate because the service must decode an
//! [`AuthTrace`] without linking the authorizer: the two are separate blobs, and depending on the
//! authorizer crate drags its `is_authorized` entry point (and a curve implementation) into the
//! service's own.
//! The token and the config stay with the authorizer, which is the only program that reads them.

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

/// A core-assignment command riding an authorization token.
///
/// The token is the only channel there is: Accumulate sees a work item's authorization *output*
/// but neither the work package nor its token, and the Coretime chain the real service takes its
/// assignments from does not exist on a test network.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub enum Command {
	/// Fill `core`'s authorizer queue with `authorizer`.
	///
	/// The hash is carried rather than derived because Accumulate cannot see an authorizer
	/// config — not even its own package's — so it has no way to build one for `para_id`.
	/// `para_id` is therefore advisory: it is what the service logs, not what it writes.
	Assign { para_id: ParaId, core: CoreIndex, authorizer: AuthorizerHash },
	/// Return `core` to the unassigned authorizer, the null-code empty-config one every core's
	/// queue holds at genesis.
	Free { core: CoreIndex },
}

/// What Is-Authorized hands to Refine and Accumulate for every work item in the package.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub struct AuthTrace {
	pub author_key: CollatorKey,
	/// The command the token carried, echoed here because the trace is the only part of the
	/// authorization the Parachain Service's Accumulate gets to see.
	pub control_command: Option<Command>,
}
