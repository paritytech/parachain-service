//! Supervisor-driven operations on a supervised JAM service (§6.5).
//!
//! The vendored host is Gray Paper 0.7.2 and has no supervisor relation, so five
//! of the six operations can only ever refuse; `create_service` really runs. See
//! DECISIONS.md D-13.

mod common;

use common::*;
use parachain_service::state::log::{
	AccumulateLog, LogEntry, ServiceCreationResult, ServiceEjectError, ServiceSolicitError,
	ServiceStoreError, ServiceSupervisorError,
};
use parachain_service_interface::{
	types::{ParaId, ServiceId, ASSET_HUB_PARA_ID},
	upward_message::{CreateServiceArgs, Target, UpwardMessage},
};

const NOW: u32 = 100;
const AH_CODE: &[u8] = b"ah-code";
/// Below JAM's `NEW_ID_BASE`, so inside the registrar-only protected range.
const WANTED_ID: ServiceId = 42;
/// A service id nothing seeds, so JAM does not know it.
const GHOST: ServiceId = 65_536;
/// A service JAM knows but this one does not supervise.
const OTHER: ServiceId = 9;

fn ah_storage() -> jam_node::vm::Storage {
	fresh_storage(|s| seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH))
}

fn ah_storage_with(service: ServiceId) -> jam_node::vm::Storage {
	fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, service, 100);
	})
}

fn create_args(desired_id: Option<ServiceId>) -> CreateServiceArgs {
	CreateServiceArgs {
		code_hash: jam_std_common::hash_raw(AH_CODE),
		len: (AH_CODE.len() as u32).into(),
		min_item_gas: 100,
		min_memo_gas: 100,
		id: 77.into(),
		desired_id,
		source_supervisor_balance: false,
		new_supervisor_balance: false,
	}
}

/// Every §6.5 message naming `service`, in design-doc order.
fn all_ops(service: ServiceId) -> Vec<UpwardMessage> {
	vec![
		UpwardMessage::Forget { target: Target::Service(service), hash: [9; 32], len: 1024.into() },
		UpwardMessage::Solicit {
			target: Target::Service(service),
			hash: [9; 32],
			len: 1024.into(),
		},
		UpwardMessage::RemoveServiceStorage { service, key: vec![1, 2] },
		UpwardMessage::EjectService { service },
		UpwardMessage::SetServiceSupervisor { service, new_supervisor: service },
	]
}

/// Replay `msgs` from Asset Hub against `storage` and return the log entries.
fn replay(
	storage: jam_node::vm::Storage,
	msgs: Vec<UpwardMessage>,
) -> (Vec<AccumulateLog>, jam_node::vm::Storage) {
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-head-1", msgs, 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let entries = para_log(&storage, ASSET_HUB_PARA_ID)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect();
	(entries, storage)
}

#[test]
fn unknown_service_errors() {
	// §6.5 checks existence before supervision, and existence is the one half a
	// GP 0.7.2 host can answer.
	let (logs, _) = replay(ah_storage(), all_ops(GHOST));

	assert_eq!(
		logs,
		vec![
			AccumulateLog::ServiceStoreFailed {
				service: GHOST,
				error: ServiceStoreError::UnknownService
			},
			AccumulateLog::ServiceSolicitFailed {
				service: GHOST,
				error: ServiceSolicitError::UnknownService
			},
			AccumulateLog::ServiceStoreFailed {
				service: GHOST,
				error: ServiceStoreError::UnknownService
			},
			AccumulateLog::ServiceEjectFailed {
				service: GHOST,
				error: ServiceEjectError::UnknownService
			},
			AccumulateLog::ServiceSupervisorFailed {
				service: GHOST,
				error: ServiceSupervisorError::UnknownService
			},
		]
	);
}

#[test]
fn known_but_unsupervised_errors() {
	// On a GP 0.7.2 host this is every service there is (D-13).
	let (logs, _) = replay(ah_storage_with(OTHER), all_ops(OTHER));

	assert_eq!(
		logs,
		vec![
			AccumulateLog::ServiceStoreFailed {
				service: OTHER,
				error: ServiceStoreError::NotSupervised
			},
			AccumulateLog::ServiceSolicitFailed {
				service: OTHER,
				error: ServiceSolicitError::NotSupervised
			},
			AccumulateLog::ServiceStoreFailed {
				service: OTHER,
				error: ServiceStoreError::NotSupervised
			},
			AccumulateLog::ServiceEjectFailed {
				service: OTHER,
				error: ServiceEjectError::NotSupervised
			},
			AccumulateLog::ServiceSupervisorFailed {
				service: OTHER,
				error: ServiceSupervisorError::NotSupervised
			},
		]
	);
}

#[test]
fn own_state_untouched_works() {
	// None of the refusals touches this service's own parachain state: no balance
	// is charged and the code reference is left alone.
	let before = ah_storage_with(OTHER);
	let info_before = para_info(&before, ASSET_HUB_PARA_ID).expect("Asset Hub is seeded");

	let (_, after) = replay(before, all_ops(OTHER));

	let info_after = para_info(&after, ASSET_HUB_PARA_ID).expect("Asset Hub is still live");
	assert_eq!(info_after.used_state_balance, info_before.used_state_balance);
	assert_eq!(info_after.validation_code, info_before.validation_code);
	assert!(registry_entry(&after, code_ref(AH_CODE))
		.is_some_and(|e| e.referencers.contains(&ASSET_HUB_PARA_ID)));
}

#[test]
fn eject_self_errors() {
	// §6.5 refuses a self-eject outright, before any lookup, so it must not be
	// reported as `UnknownService` nor as `NotSupervised`.
	let (logs, _) = replay(ah_storage(), vec![UpwardMessage::EjectService { service: SVC }]);

	assert_eq!(
		logs,
		vec![AccumulateLog::ServiceEjectFailed {
			service: SVC,
			error: ServiceEjectError::TargetIsSelf
		}]
	);
}

#[test]
fn set_supervisor_unknown_new_supervisor_errors() {
	// The new supervisor's absence outranks this service's own lack of rights.
	let msgs = vec![UpwardMessage::SetServiceSupervisor { service: OTHER, new_supervisor: GHOST }];

	let (logs, _) = replay(ah_storage_with(OTHER), msgs);

	assert_eq!(
		logs,
		vec![AccumulateLog::ServiceSupervisorFailed {
			service: OTHER,
			error: ServiceSupervisorError::UnknownNewSupervisor
		}]
	);
}

#[test]
fn create_works() {
	// The one §6.5 operation the vendored host can execute. JAM assigns the id
	// and records this service as the new one's parent.
	let msgs = vec![UpwardMessage::CreateService(create_args(None))];

	let (logs, storage) = replay(ah_storage(), msgs);

	let [AccumulateLog::ServiceCreation { id, result: ServiceCreationResult::Created(new_id) }] =
		logs[..]
	else {
		panic!("expected a successful ServiceCreation, got {logs:?}")
	};
	assert_eq!(id.0, 77, "the caller's own handle is echoed back");
	let created = storage.service(new_id).expect("JAM created the service");
	assert_eq!(created.parent_service, SVC);
	assert_eq!(created.code_hash.0, jam_std_common::hash_raw(AH_CODE));
}

#[test]
fn create_desired_id_works() {
	// §3: the service is the registrar, so a `desired_id` inside the protected
	// range is honoured verbatim.
	let msgs = vec![UpwardMessage::CreateService(create_args(Some(WANTED_ID)))];

	let (logs, storage) = replay(ah_storage(), msgs);

	assert_eq!(
		logs,
		vec![AccumulateLog::ServiceCreation {
			id: 77.into(),
			result: ServiceCreationResult::Created(WANTED_ID)
		}]
	);
	assert!(storage.service(WANTED_ID).is_some());
}

#[test]
fn create_id_taken_errors() {
	// A protected index already in use is refused, and the sitting service is
	// left untouched.
	let storage = ah_storage_with(WANTED_ID);
	let taken = storage.service(WANTED_ID).expect("seeded");
	let msgs = vec![UpwardMessage::CreateService(create_args(Some(WANTED_ID)))];

	let (logs, storage) = replay(storage, msgs);

	assert_eq!(
		logs,
		vec![AccumulateLog::ServiceCreation {
			id: 77.into(),
			result: ServiceCreationResult::IdTaken
		}]
	);
	assert_eq!(storage.service(WANTED_ID).expect("untouched").code_hash, taken.code_hash);
}

#[test]
fn create_supervisor_balance_errors() {
	// Either selector needs a GP >= 0.8 two-balance service, so the creation is
	// refused rather than silently funded from the wrong balance (D-13).
	for args in [
		CreateServiceArgs { source_supervisor_balance: true, ..create_args(None) },
		CreateServiceArgs { new_supervisor_balance: true, ..create_args(None) },
	] {
		let (logs, _) = replay(ah_storage(), vec![UpwardMessage::CreateService(args)]);

		assert_eq!(
			logs,
			vec![AccumulateLog::ServiceCreation {
				id: 77.into(),
				result: ServiceCreationResult::CannotAfford
			}]
		);
	}
}

#[test]
fn dropped_from_non_asset_hub_works() {
	// §4.3: the accumulate-side re-check drops the whole candidate when a
	// non-Asset-Hub para carries one of these, so its head never moves.
	const PARA: ParaId = ParaId(1000);
	const CODE: &[u8] = b"para-1000-code";

	for msg in all_ops(OTHER)
		.into_iter()
		.chain([UpwardMessage::CreateService(create_args(None))])
	{
		let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));

		let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);
		let (_, after, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

		let info = para_info(&after, PARA).expect("para is still live");
		assert_eq!(info.head_data.to_vec(), b"genesis".to_vec(), "candidate must be dropped");
	}
}
