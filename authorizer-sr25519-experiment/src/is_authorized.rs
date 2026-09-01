use super::aura;
use codec::{DecodeAll, Encode};
use jam_pvm_common::is_authorized::{auth_token, refine_context, work_package};
use jam_types::{AuthTrace, CoreIndex};

#[derive(Debug)]
pub enum AuthorizationError {
	UndecodableAuthConfig,
	UndecodableAuthToken,
	InvalidWorkItemCount,
	/// A work item targets a service other than the configured Parachain
	/// Service — para-specific coretime must not authorize other JAM work
	/// (SPEC_GAPS #7).
	WrongTargetService,
	/// `collator_set_size == 0` — no collator could ever be selected.
	ZeroCollatorSetSize,
	/// `slot_duration == 0` — the round-robin index would divide by zero.
	ZeroSlotDuration,
	BadAuthToken(aura::TokenError),
}

pub fn is_authorized(_core: CoreIndex) -> Result<AuthTrace, AuthorizationError> {
	let package = work_package();
	assert!(
		package.items.len() > 0,
		"work packages need to have at least one item (see Gray Paper)"
	);
	let raw_config = &package.authorizer.config;

	let config = aura::AuthConfig::decode_all(&mut &raw_config[..])
		.map_err(|_| AuthorizationError::UndecodableAuthConfig)?;

	if config.para_ids.len() != package.items.len() {
		return Err(AuthorizationError::InvalidWorkItemCount);
	}
	if package.items.iter().any(|item| item.service != config.parachain_service) {
		return Err(AuthorizationError::WrongTargetService);
	}
	if config.collator_set_size == 0 {
		return Err(AuthorizationError::ZeroCollatorSetSize);
	}
	if config.slot_duration == 0 {
		return Err(AuthorizationError::ZeroSlotDuration);
	}

	let raw_token = &auth_token().0;
	let token = aura::AuthToken::decode_all(&mut &raw_token[..])
		.map_err(|_| AuthorizationError::UndecodableAuthToken)?;

	// §7.1 step 4: the slot-selected collator. FIXME: the design says to read
	// the *anchor* timeslot from the refinement context, but the Gray Paper's
	// RefineContext exposes only the lookup-anchor slot — the anchor's slot is
	// not available in-core. Using `lookup_anchor_slot` here lets a collator
	// pick any lookup anchor mapping to its own index; needs upstreaming.
	let slot = refine_context().lookup_anchor_slot;
	let collator_index = aura::expected_collator_index(slot, &config);

	let trace = token
		.try_into_trace(&config, &package, collator_index)
		.map_err(AuthorizationError::BadAuthToken)?;

	Ok(AuthTrace(trace.encode()))
}
