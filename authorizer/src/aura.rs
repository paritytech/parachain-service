//! Shared AURA protocol and the local development sentinel.

pub use cumulus_aura_authorizer::*;

/// Development sentinel that bypasses signature, membership, item-count, and slot checks.
/// Target-service validation still applies. This permits control commands on parked cores
/// whose config has no para IDs. The sentinel preserves the three-field token encoding.
///
/// FIXME: Anyone can submit unsigned control commands with this key. Remove or isolate
/// this bypass before production use.
pub const SUDO_KEY: CollatorKey = [0xFF; 32];

/// Hash the service's vendored JAM work-package format without converting it to the
/// older published format. In particular, retain every refinement-context field.
pub fn signable_work_package_hash(package: &jam_types::WorkPackage) -> primitive_types::H256 {
	use jam_types::Encode as JamEncode;
	let payload = JamEncode::encode(&(
		&package.auth_code_host,
		&package.authorizer.code_hash,
		&package.context,
		&package.authorizer.config,
		&package.items,
	));
	hash_signing_payload(&payload)
}

/// Apply shared AURA validation to the service's vendored JAM work-package format.
pub fn authorize<S: SignatureVerifier>(
	config: &AuthConfig,
	token: &AuthToken,
	package: &jam_types::WorkPackage,
	slot: jam_types::Slot,
) -> Result<AuthTrace, AuthorizationError> {
	authorize_hash::<S>(config, token, signable_work_package_hash(package), slot)
}
