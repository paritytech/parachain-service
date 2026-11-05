use super::aura::{self, AuthConfig, AuthToken, SignatureVerifier};
use codec::{DecodeAll, Encode};
use jam_pvm_common::is_authorized::{auth_token, refine_context, work_package};
use jam_types::{AuthTrace, CoreIndex};

#[derive(Debug)]
pub(crate) enum AuthorizationError {
	UndecodableAuthConfig,
	UndecodableAuthToken,
	BadAuthorization(aura::AuthorizationError),
}

pub(crate) fn is_authorized<S: SignatureVerifier>(
	_core: CoreIndex,
) -> Result<AuthTrace, AuthorizationError> {
	let package = work_package();
	assert!(
		package.items.len() > 0,
		"work packages need to have at least one item (see Gray Paper)"
	);

	let config = AuthConfig::decode_all(&mut &package.authorizer.config[..])
		.map_err(|_| AuthorizationError::UndecodableAuthConfig)?;
	let token = AuthToken::decode_all(&mut &auth_token().0[..])
		.map_err(|_| AuthorizationError::UndecodableAuthToken)?;

	// §7.1 selects the collator using the recent anchor's timeslot.
	let slot = refine_context().anchor_slot;

	let trace = aura::AuthTrace::try_from_token::<S>(&config, &token, &package, slot)
		.map_err(AuthorizationError::BadAuthorization)?;
	Ok(AuthTrace(trace.encode()))
}

#[cfg(test)]
mod tests;
