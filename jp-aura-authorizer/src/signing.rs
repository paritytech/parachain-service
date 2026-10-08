use alloc::vec::Vec;
use jam_types::{Encode as JamEncode, WorkPackage};
use primitive_types::H256;
use sp_crypto_hashing::blake2_256;

/// Domain separator for the token-free work-package hash signed by AURA collators.
pub const WORK_PACKAGE_SIGN_CTX: &[u8] = b"jam:parachain-service:aura:work-package:v1";

/// Hash of a work-package that can be signed by AURA collators.
///
/// JAM-encodes the code host, authorizer code hash, refinement context, authorizer config,
/// and work items after [`WORK_PACKAGE_SIGN_CTX`], then hashes them with Blake2b-256.
/// The authorization token is excluded because it contains the signature.
pub fn signable_work_package_hash(package: &WorkPackage) -> H256 {
	let mut signable = Vec::new();
	signable.extend_from_slice(WORK_PACKAGE_SIGN_CTX);
	JamEncode::encode_to(
		&(
			&package.auth_code_host,
			&package.authorizer.code_hash,
			&package.context,
			&package.authorizer.config,
			&package.items,
		),
		&mut signable,
	);

	H256::from(blake2_256(&signable))
}
