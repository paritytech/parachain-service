use super::aura::{self, AuthConfig, AuthToken, SignatureScheme};
use codec::{DecodeAll, Encode};
use jam_pvm_common::is_authorized::{auth_token, refine_context, work_package};
use jam_types::{AuthTrace, CoreIndex, Slot, WorkPackage};

#[derive(Debug)]
pub enum AuthorizationError {
	UndecodableAuthConfig,
	UndecodableAuthToken,
	InvalidWorkItemCount,
	/// A work item targets a service other than the configured Parachain
	/// Service — para-specific coretime must not authorize other JAM work.
	WrongTargetService,
	/// `collator_set_size == 0` — no collator could ever be selected.
	ZeroCollatorSetSize,
	/// `slot_duration == 0` — the round-robin index would divide by zero.
	ZeroSlotDuration,
	BadAuthToken(aura::TokenError),
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

/// The authorization decision itself, over everything [`is_authorized`] reads from the host.
///
/// Split out because it is the contract with whoever builds a token, and the cross-scheme
/// contract tests can only run the *real* decision if it is reachable without a PVM host. Nothing
/// here makes a host call.
pub fn authorize<S: SignatureScheme>(
	config: &AuthConfig,
	token: &AuthToken,
	package: &WorkPackage,
	lookup_anchor_slot: Slot,
) -> Result<aura::AuthTrace, AuthorizationError> {
	// Para-specific coretime must not be spent on other JAM work, whatever a package carries.
	if package.items.iter().any(|item| item.service != config.parachain_service) {
		return Err(AuthorizationError::WrongTargetService);
	}
	if config.para_ids.len() != package.items.len() {
		return Err(AuthorizationError::InvalidWorkItemCount);
	}
	if config.collator_set_size == 0 {
		return Err(AuthorizationError::ZeroCollatorSetSize);
	}
	if config.slot_duration == 0 {
		return Err(AuthorizationError::ZeroSlotDuration);
	}

	let collator_index = aura::expected_collator_index(lookup_anchor_slot, config);
	token
		.try_into_trace::<S>(config, package, collator_index)
		.map_err(AuthorizationError::BadAuthToken)
}
