use super::aura::{self, AuthConfig, AuthToken, SignatureScheme};
use codec::{DecodeAll, Encode};
use jam_pvm_common::is_authorized::{auth_token, refine_context, work_package};
use jam_types::{AuthTrace, CoreIndex, Slot, WorkPackage};

#[derive(Debug)]
pub enum AuthorizationError {
	UndecodableAuthConfig,
	UndecodableAuthToken,
	/// Number of work items does not match the number of para IDs.
	InvalidWorkItemCount,
	/// A work item does not target the Parachain Service.
	WrongTargetService,
	BadAuthorization(aura::AuthorizationError),
}

pub fn is_authorized<S: SignatureScheme>(
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

	// §7.1 step 4 wants the slot to select the collator with. FIXME: the design says to read the
	// *anchor* timeslot from the refinement context, but the Gray Paper's RefineContext exposes
	// only the lookup-anchor slot — the anchor's slot is not available in-core. Using
	// `lookup_anchor_slot` here lets a collator pick any lookup anchor mapping to its own index;
	// needs upstreaming.
	let slot = refine_context().lookup_anchor_slot;

	Ok(AuthTrace(authorize::<S>(&config, &token, &package, slot)?.encode()))
}

/// Validate a package without host calls, using the supplied config, token, and slot.
pub fn authorize<S: SignatureScheme>(
	config: &AuthConfig,
	token: &AuthToken,
	package: &WorkPackage,
	lookup_anchor_slot: Slot,
) -> Result<aura::AuthTrace, AuthorizationError> {
	// Deliberately outside the sudo bypass: para-specific coretime must not be spent on other
	// JAM work whatever a package carries (SPEC_GAPS #7).
	if package.items.iter().any(|item| item.service != config.parachain_service) {
		return Err(AuthorizationError::WrongTargetService);
	}

	// Permit control commands on parked cores. FIXME: remove the development bypass; see SUDO_KEY.
	if token.key == aura::SUDO_KEY {
		return Ok(aura::AuthTrace { author_key: token.key, sudo: true });
	}

	if config.para_ids.len() != package.items.len() {
		return Err(AuthorizationError::InvalidWorkItemCount);
	}
	aura::authorize::<S>(config, token, package, lookup_anchor_slot)
		.map_err(AuthorizationError::BadAuthorization)
}
