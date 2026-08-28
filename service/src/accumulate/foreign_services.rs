//! Supervisor-driven operations on a supervised JAM service (spec §6.5).
//!
//! These act on JAM account state rather than on this service's own storage, so
//! nothing here touches `ParaInfo`, the preimage registry or the KV store; the
//! only trace they leave in service state is the log entry each one produces.
//!
//! # Host support (DECISIONS.md D-13)
//!
//! §6.5 presupposes a Gray Paper >= 0.8 host: a per-service *supervisor* link the
//! supervisor may act through, plus foreign `solicit`/`forget` and a foreign
//! storage write. The vendored PolkaJAM host is GP 0.7.2 and has none of them.
//! `ServiceInfo::parent_service` records a creator, but no host call is gated on
//! it and nothing can assert or transfer supervision, so the Parachain Service is
//! never any service's effective supervisor.
//!
//! Every operation therefore accepts the full spec shape on the wire and refuses
//! with the error the design itself assigns — `NotSupervised`, once existence has
//! been ruled out — exactly as D-11 does for `transfer_out`'s unsupported shapes.
//! `create_service` is the one operation the host can express, and it really runs.
//! FIXME: revisit every refusal below once the host exposes GP >= 0.8 supervision.

use crate::state::log::{
	AccumulateLog, ServiceCreationResult, ServiceEjectError, ServiceSolicitError,
	ServiceStoreError, ServiceSupervisorError,
};
use alloc::vec::Vec;
use jam_pvm_common::{
	accumulate::{create_service as jam_create_service, service_info},
	ApiError,
};
use jam_types::CodeHash;
use parachain_service_interface::{types::ServiceId, upward_message::CreateServiceArgs};

/// The existence half of every §6.5 precondition, and the only half this host
/// can actually answer.
fn exists(service: ServiceId) -> bool {
	service_info(service).is_some()
}

/// A `Service`-targeted `forget`, or a `remove_service_storage`. Both share the
/// same existence-then-supervision precondition and the same log entry.
pub fn store_op(service: ServiceId, logs: &mut Vec<AccumulateLog>) {
	let error = if exists(service) {
		ServiceStoreError::NotSupervised
	} else {
		ServiceStoreError::UnknownService
	};
	logs.push(AccumulateLog::ServiceStoreFailed { service, error });
}

/// A `Service`-targeted `solicit`.
pub fn solicit(service: ServiceId, logs: &mut Vec<AccumulateLog>) {
	let error = if exists(service) {
		ServiceSolicitError::NotSupervised
	} else {
		ServiceSolicitError::UnknownService
	};
	logs.push(AccumulateLog::ServiceSolicitFailed { service, error });
}

/// An `eject_service`. `TargetIsSelf` is checked before existence, as §6.5
/// orders it: naming ourselves is refused outright rather than looked up.
pub fn eject(service: ServiceId, self_id: ServiceId, logs: &mut Vec<AccumulateLog>) {
	let error = if service == self_id {
		ServiceEjectError::TargetIsSelf
	} else if !exists(service) {
		ServiceEjectError::UnknownService
	} else {
		// The host's `eject` needs the target to have zombified itself naming us
		// as ejector, which a supervisor cannot do on its behalf, so the
		// unilateral form §6.5 specifies is unreachable regardless of emptiness.
		ServiceEjectError::NotSupervised
	};
	logs.push(AccumulateLog::ServiceEjectFailed { service, error });
}

/// A `set_service_supervisor`. Both services must exist before supervision is
/// considered, and the new supervisor's absence outranks our own lack of rights.
/// Naming `service` itself — §6.5's "set it free" form — needs no special case:
/// it exists by the first check.
pub fn set_supervisor(
	service: ServiceId,
	new_supervisor: ServiceId,
	logs: &mut Vec<AccumulateLog>,
) {
	let error = if !exists(service) {
		ServiceSupervisorError::UnknownService
	} else if !exists(new_supervisor) {
		ServiceSupervisorError::UnknownNewSupervisor
	} else {
		ServiceSupervisorError::NotSupervised
	};
	logs.push(AccumulateLog::ServiceSupervisorFailed { service, error });
}

/// A `create_service`: the one §6.5 operation the vendored host can execute.
///
/// JAM records this service as the new one's parent, solicits `code_hash` in the
/// new service's own store, and funds it with its threshold balance out of this
/// service's balance. `desired_id` is honoured only while this service holds
/// JAM's `registrar` privilege and the index is inside the protected range;
/// outside it the host silently allocates instead (D-13).
pub fn create(args: CreateServiceArgs, logs: &mut Vec<AccumulateLog>) {
	let CreateServiceArgs {
		code_hash,
		len,
		min_item_gas,
		min_memo_gas,
		id,
		desired_id,
		source_supervisor_balance,
		new_supervisor_balance,
	} = args;

	// A single balance per service leaves both selectors inexpressible, and §6.5
	// defines no error for one. Refusing is the conservative reading: reusing
	// `CannotAfford` at least tells the caller the funding did not happen.
	// FIXME: the design needs an error for an inexpressible balance selector
	// (same gap as F-14).
	if source_supervisor_balance || new_supervisor_balance {
		logs.push(AccumulateLog::ServiceCreation {
			id,
			result: ServiceCreationResult::CannotAfford,
		});
		return;
	}

	// `deposit_offset` stays 0: a non-zero offset grants gratis storage and needs
	// JAM's `manager` privilege, which §3 does not claim for this service.
	let result = match jam_create_service(
		&CodeHash(code_hash),
		len.0 as usize,
		min_item_gas,
		min_memo_gas,
		0,
		desired_id,
	) {
		Ok(new_id) => ServiceCreationResult::Created(new_id),
		Err(ApiError::StorageFull) => ServiceCreationResult::IdTaken,
		// `NoCash` is the only other reachable failure: `deposit_offset` is 0, so
		// the host's `manager` check cannot fire, and the code hash always reads
		// back, so neither can its memory checks.
		Err(_) => ServiceCreationResult::CannotAfford,
	};
	logs.push(AccumulateLog::ServiceCreation { id, result });
}
