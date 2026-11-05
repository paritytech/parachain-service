//! Block authorization: the JAM authorizer authorizes the author of a parachain block.
//!
//! JAM authorizes a work package by running its authorizer's `is_authorized` entry point. So that
//! the parachain's view of who may author a block cannot drift away from JAM's (issue #62), the
//! parachain runs the very same authorizer on its blocks: every block carries a
//! [`BlockAuthorization`] in its seal, and the runtime checks that the authorizer is in an auth
//! pool at the block's anchor and runs the authorizer's second entry point,
//! `authorize_block_ext`, on the encoded [`BlockAuthorizationParams`].

use crate::{
	authorizer::{authorizer_hash, AuthConfigBlob, Authorizer, AuthorizerHash},
	types::ParaId,
	StateKey,
};
use alloc::vec::Vec;
use codec::{Decode, Encode};

/// Consensus engine id of the seal that carries a [`BlockAuthorization`].
pub const BLOCK_AUTHORIZATION_ENGINE_ID: [u8; 4] = *b"jaut";

/// The state key of JAM's auth pools, `C(1)`.
pub const AUTH_POOLS_STATE_KEY: StateKey = {
	let mut key = [0u8; 31];
	key[0] = 1;
	key
};

/// The program counter of the authorizer's `authorize_block_ext` entry point: the second entry of
/// a `block-authorizer` blob's dispatch table, as `accumulate` is for a service.
pub const AUTHORIZE_BLOCK_ENTRY_PC: u32 = 5;

/// What a block's seal carries: the authorizer to run and the token for it.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct BlockAuthorization {
	/// The service holding the authorizer code as a preimage.
	pub auth_code_host: u32,
	/// The authorizer code hash.
	pub code_hash: [u8; 32],
	/// The authorizer config, starting with the `Vec<ParaId>` every parachain authorizer config
	/// starts with (§7.1).
	pub config: Vec<u8>,
	/// Opaque to everyone but the authorizer.
	pub token: Vec<u8>,
}

impl BlockAuthorization {
	/// The hash an auth pool has to hold for this authorizer to be allowed.
	pub fn authorizer_hash(&self) -> AuthorizerHash {
		authorizer_hash(&Authorizer {
			code_hash: self.code_hash.into(),
			config: AuthConfigBlob(self.config.clone()),
		})
	}

	/// The paras the config names, `None` if it does not start with a `Vec<ParaId>`.
	pub fn para_ids(&self) -> Option<Vec<ParaId>> {
		Vec::<ParaId>::decode(&mut &self.config[..]).ok()
	}
}

/// The input of the authorizer's `authorize_block_ext` entry point.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct BlockAuthorizationParams {
	pub config: Vec<u8>,
	pub token: Vec<u8>,
	/// Hash of the block header without its seal.
	pub block_hash: [u8; 32],
	/// The JAM slot the authorizer selects the expected author with.
	pub slot: u32,
}

/// Whether any of the encoded auth pools, the value under [`AUTH_POOLS_STATE_KEY`], holds `hash`.
///
/// The pools are one length-prefixed hash list per core. The core count is a protocol parameter,
/// so this walks the encoding to its end instead. `None` if `pools` is not such an encoding.
pub fn auth_pools_contain(mut pools: &[u8], hash: &AuthorizerHash) -> Option<bool> {
	let mut found = false;
	while !pools.is_empty() {
		let pool = <Vec<AuthorizerHash> as jam_types::Decode>::decode(&mut pools).ok()?;
		found |= pool.contains(hash);
	}
	Some(found)
}

#[cfg(test)]
mod tests {
	use super::*;
	use jam_types::{BoundedVec, Encode as _};

	fn pools(per_core: &[&[AuthorizerHash]]) -> Vec<u8> {
		let mut pools = jam_std_common::AuthPools::default();
		for (core, hashes) in per_core.iter().enumerate() {
			pools[core] = BoundedVec::truncate_from(hashes.to_vec());
		}
		pools.encode()
	}

	#[test]
	fn auth_pools_state_key_works() {
		let theirs: jam_std_common::StorageKey = jam_std_common::SystemKey::AuthPools.into();
		assert_eq!(AUTH_POOLS_STATE_KEY, *theirs);
	}

	#[test]
	fn auth_pools_contain_works() {
		let (a, b, c) = (AuthorizerHash([1; 32]), AuthorizerHash([2; 32]), AuthorizerHash([3; 32]));
		let encoded = pools(&[&[a, a], &[], &[b]]);

		assert_eq!(auth_pools_contain(&encoded, &a), Some(true));
		assert_eq!(auth_pools_contain(&encoded, &b), Some(true));
		assert_eq!(auth_pools_contain(&encoded, &c), Some(false));
	}

	#[test]
	fn auth_pools_contain_malformed_errors() {
		// The first pool's only hash, cut in half.
		let encoded = pools(&[&[AuthorizerHash([1; 32])]]);
		assert_eq!(auth_pools_contain(&encoded[..17], &AuthorizerHash([1; 32])), None);
	}

	#[test]
	fn authorizer_hash_matches_authorizer_works() {
		let auth = BlockAuthorization {
			auth_code_host: 7,
			code_hash: [9; 32],
			config: alloc::vec![0, 1, 2],
			token: alloc::vec![3],
		};
		let authorizer =
			Authorizer { code_hash: [9; 32].into(), config: AuthConfigBlob(alloc::vec![0, 1, 2]) };
		assert_eq!(auth.authorizer_hash(), authorizer_hash(&authorizer));
	}

	#[test]
	fn para_ids_works() {
		let config = (alloc::vec![ParaId::from(100u32)], 5u32).encode();
		let auth = BlockAuthorization {
			auth_code_host: 0,
			code_hash: [0; 32],
			config,
			token: alloc::vec![],
		};
		assert_eq!(auth.para_ids(), Some(alloc::vec![ParaId::from(100u32)]));

		let auth = BlockAuthorization { config: alloc::vec![0xff], ..auth };
		assert_eq!(auth.para_ids(), None);
	}
}
