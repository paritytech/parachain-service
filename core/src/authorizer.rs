//! Authorizer-hash contract (contract 2).
//!
//! The Coretime chain (writes the queue), the collator (scans and signs), and the PVF
//! (decodes the config) must hash byte-identical blobs. The layout, pinned by
//! `jam_types::Authorizer::{with_concat, hash}`, is `blake2b-256(code_hash ‖ config)` —
//! a raw concatenation of the 32 code-hash bytes and the raw config blob. No domain
//! separator, no SCALE struct wrapper.

extern crate alloc;

pub use jam_types::{AuthConfig as AuthConfigBlob, Authorizer, AuthorizerHash, CodeHash};

/// Compute the authorizer hash: `blake2b-256(code_hash ‖ config)`.
pub fn authorizer_hash(authorizer: &Authorizer) -> AuthorizerHash {
	authorizer.hash(blake2b_256)
}

fn blake2b_256(data: &[u8]) -> jam_types::Hash {
	let mut hash = [0u8; 32];
	hash.copy_from_slice(blake2b_simd::Params::new().hash_length(32).hash(data).as_bytes());
	hash
}

/// Code hash of polkajam's null authorizer (accepts anything). Dev/testnet genesis fills
/// every core's queue with this authorizer.
pub const NULL_AUTHORIZER_CODE_HASH: [u8; 32] = jam_null_authorizer_bin::HASH;

/// The hardcoded phase-1 authorizer: the null authorizer with an empty config.
pub fn fixed_authorizer() -> Authorizer {
	Authorizer {
		code_hash: NULL_AUTHORIZER_CODE_HASH.into(),
		config: AuthConfigBlob(alloc::vec![]),
	}
}

/// `authorizer_hash(fixed_authorizer())`, precomputed. This is the hash the phase-1 core
/// scan looks for in the authorizer queues.
pub const FIXED_AUTHORIZER_HASH: AuthorizerHash = AuthorizerHash([
	35, 87, 66, 111, 35, 19, 85, 154, 39, 29, 103, 130, 220, 0, 25, 123, 55, 159, 121, 203, 227,
	198, 161, 231, 47, 97, 247, 181, 146, 197, 9, 248,
]);

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fixed_authorizer_hash_matches_helper() {
		assert_eq!(authorizer_hash(&fixed_authorizer()), FIXED_AUTHORIZER_HASH);
	}

	#[test]
	fn authorizer_hash_is_raw_concat_not_scale() {
		let authorizer = Authorizer {
			code_hash: [7u8; 32].into(),
			config: AuthConfigBlob(alloc::vec![1, 2, 3]),
		};
		let concat: alloc::vec::Vec<u8> = [&[7u8; 32][..], &[1, 2, 3][..]].concat();
		let expected = AuthorizerHash(blake2b_256(&concat));
		assert_eq!(authorizer_hash(&authorizer), expected);
	}
}
