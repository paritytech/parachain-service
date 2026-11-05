//! The second entry point: authorizing the author of a parachain block.
//!
//! The parachain runtime runs this on every block (issue #62), so the parachain and JAM decide who
//! may author with the very same code and config. It gets no host calls, so everything comes in
//! the encoded [`BlockAuthorizationParams`].

use super::aura::{self, AuthConfig, AuthToken, SignatureVerifier};
use codec::DecodeAll;
use parachain_service_core::{blake2_256, block_authorization::BlockAuthorizationParams};
use primitive_types::H256;

/// Domain separator for the block hash signed by AURA collators.
///
/// Distinct from `WORK_PACKAGE_SIGN_CTX`, so a signature over a work package never authorizes a
/// block and the other way around.
pub const BLOCK_SIGN_CTX: &[u8] = b"jam:parachain-service:aura:block:v1";

#[derive(Debug)]
pub enum BlockAuthorizationError {
	UndecodableParams,
	UndecodableAuthConfig,
	UndecodableAuthToken,
	BadAuthorization(aura::AuthorizationError),
}

/// The hash a collator signs to authorize the block whose seal-less header hashes to `block_hash`.
pub fn signable_block_hash(block_hash: &[u8; 32]) -> H256 {
	H256(blake2_256(&[BLOCK_SIGN_CTX, &block_hash[..]].concat()))
}

/// Authorize the block described by the encoded [`BlockAuthorizationParams`] in `input`.
pub fn authorize_block<S: SignatureVerifier>(
	input: &[u8],
) -> Result<aura::AuthTrace, BlockAuthorizationError> {
	let params = BlockAuthorizationParams::decode_all(&mut &input[..])
		.map_err(|_| BlockAuthorizationError::UndecodableParams)?;
	let config = AuthConfig::decode_all(&mut &params.config[..])
		.map_err(|_| BlockAuthorizationError::UndecodableAuthConfig)?;
	let token = AuthToken::decode_all(&mut &params.token[..])
		.map_err(|_| BlockAuthorizationError::UndecodableAuthToken)?;

	aura::authorize_hash::<S>(&config, &token, signable_block_hash(&params.block_hash), params.slot)
		.map_err(BlockAuthorizationError::BadAuthorization)
}
