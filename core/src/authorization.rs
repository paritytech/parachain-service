//! Local authorization wire types, including the parasim development wrapper.

use codec::{Decode, Encode, MaxEncodedLen};

pub use jp_aura_authorizer::{AuthTrace, CollatorKey, CollatorSignature};

/// Local authorization output carrying the parasim development-control flag.
///
/// SCALE encodes the nested AURA trace inline: the author key is followed by `sudo`,
/// preserving the existing local wire format.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, MaxEncodedLen)]
pub struct DevelopmentAuthTrace {
	pub aura: jp_aura_authorizer::AuthTrace,
	/// The local adapter admitted a development control package through its sentinel bypass.
	pub sudo: bool,
}
