//! SCALE-encoded authorization output shared by verifier and service.

use codec::{Decode, Encode, MaxEncodedLen};

/// Raw public key bytes for either supported signature scheme.
pub type CollatorKey = [u8; 32];

/// A collator's signature over an authorization token's signing payload, raw 64 bytes.
pub type CollatorSignature = [u8; 64];

/// What Is-Authorized hands to Refine and Accumulate for every work item in the package.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub struct AuthTrace {
	pub author_key: CollatorKey,
	/// Marks development control packages, which Refine interprets as commands rather than blocks.
	pub sudo: bool,
}
