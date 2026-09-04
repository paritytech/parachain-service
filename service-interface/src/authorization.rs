//! The authorization wire types the `service` and `authorizer` programs both speak.
//!
//! They live here rather than in the authorizer crate because the service must decode an
//! [`AuthTrace`] without linking the authorizer: the two are separate blobs, and depending on the
//! authorizer crate drags its `is_authorized` entry point (and a curve implementation) into the
//! service's own.
//! The token and the config stay with the authorizer, which is the only program that reads them.

use codec::{Decode, Encode, MaxEncodedLen};

/// A collator public key: the raw 32 bytes, whichever curve they are on.
///
/// Both aura schemes have the same shape here, which is what lets everything but the verifier
/// itself stay scheme-blind. Which curve a core's collators sign on is settled by the authorizer
/// blob its queue commits to.
pub type CollatorKey = [u8; 32];

/// A collator's signature over an authorization token's signing payload, raw 64 bytes.
pub type CollatorSignature = [u8; 64];

/// What Is-Authorized hands to Refine and Accumulate for every work item in the package.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub struct AuthTrace {
	pub author_key: CollatorKey,
}
