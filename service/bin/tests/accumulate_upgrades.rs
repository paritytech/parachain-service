//! Code-upgrade lifecycle (§5.2) and service self-upgrade (§5.4).

mod common;

use common::*;
use parachain_service::{
	constants::UPGRADE_TIMEOUT_TIMESLOTS,
	state::log::{AccumulateLog, LogEntry},
	state_balance::preimage_footprint,
};
use parachain_service_interface::{
	types::{ParaId, ASSET_HUB_PARA_ID},
	upward_message::UpwardMessage,
};

const NOW: u32 = 100;
const PARA: ParaId = ParaId(1000);
const CODE: &[u8] = b"para-1000-code";
const NEW_CODE: &[u8] = b"para-1000-code-v2";

fn accumulate_logs(storage: &jam_node::vm::Storage, para: ParaId) -> Vec<AccumulateLog> {
	para_log(storage, para)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect()
}

fn request_upgrade_block(
	storage: jam_node::vm::Storage,
) -> (jam_node::vm::Storage, parachain_service::work_digest::ValidationCodeRef) {
	let new_ref = code_ref(NEW_CODE);
	let msg = UpwardMessage::RequestCodeUpgrade { hash: new_ref.hash, len: new_ref.len.into() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);
	(storage, new_ref)
}

#[test]
fn request_works() {
	// §5.2 phase 2: pending armed with a deadline, new code solicited + charged.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;

	let (storage, new_ref) = request_upgrade_block(storage);

	let info = para_info(&storage, PARA).unwrap();
	let (pending, deadline) = info.pending_upgrade.as_ref().expect("pending armed");
	assert_eq!(pending.code_ref, new_ref);
	assert!(!pending.pinned);
	assert_eq!(*deadline, NOW + UPGRADE_TIMEOUT_TIMESLOTS);
	assert_eq!(info.used_state_balance, used_before + preimage_footprint(new_ref.len));
	assert!(registry_entry(&storage, new_ref).is_some_and(|e| e.referencers.contains(&PARA)));
	// The old code stays active during the transition window.
	assert_eq!(info.validation_code.as_ref().unwrap().code_ref, code_ref(CODE));
}

#[test]
fn activation_works() {
	// §5.2 phase 5(a): the first candidate validated with the new code activates
	// it and releases the old code via the two-step forget.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let (storage, new_ref) = request_upgrade_block(storage);

	let digest = ok_digest(PARA, NEW_CODE, b"head-1", b"head-2", vec![], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW + 10);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.validation_code.as_ref().unwrap().code_ref, new_ref);
	assert!(info.pending_upgrade.is_none());
	assert_eq!(&info.head_data[..], b"head-2");
	// The old code was provided, so its release is two-step: still referenced,
	// follow-up logged.
	assert!(registry_entry(&storage, code_ref(CODE)).is_some());
	assert!(matches!(accumulate_logs(&storage, PARA)[..], [AccumulateLog::ForgetAgainAt { .. }]));
}

#[test]
fn old_code_during_transition_works() {
	// §5.2 phase 4: candidates using the old code still enact while pending.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let (storage, new_ref) = request_upgrade_block(storage);

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW + 10);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(&info.head_data[..], b"head-2");
	assert_eq!(info.validation_code.as_ref().unwrap().code_ref, code_ref(CODE));
	assert!(info.pending_upgrade.is_some(), "pending stays armed");
	let _ = new_ref;
}

#[test]
fn timeout_reap_works() {
	// §5.2 phase 5(b): past the deadline, the next candidate reaps the pending
	// upgrade. The never-provided new code drops in one forget, refunding fully.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let used_original = para_info(&storage, PARA).unwrap().used_state_balance;
	let (storage, _) = request_upgrade_block(storage);

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![], 0);
	let (_, storage, _) =
		run_block(storage, vec![work_item(&digest)], NOW + UPGRADE_TIMEOUT_TIMESLOTS);

	let info = para_info(&storage, PARA).unwrap();
	assert!(info.pending_upgrade.is_none(), "timed-out upgrade reaped");
	assert_eq!(&info.head_data[..], b"head-2", "the candidate itself enacted");
	assert_eq!(info.used_state_balance, used_original, "unprovided code fully refunded");
	assert!(registry_entry(&storage, code_ref(NEW_CODE)).is_none());
}

#[test]
fn supersede_works() {
	// §5.2 phase 2: a different in-flight upgrade is superseded.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let (storage, _) = request_upgrade_block(storage);

	let third_ref = code_ref(b"para-1000-code-v3");
	let msg = UpwardMessage::RequestCodeUpgrade { hash: third_ref.hash, len: third_ref.len.into() };
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![msg], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW + 1);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.pending_upgrade.as_ref().unwrap().0.code_ref, third_ref);
	// The superseded (unprovided) v2 code was dropped outright.
	assert!(registry_entry(&storage, code_ref(NEW_CODE)).is_none());
}

#[test]
fn service_upgrade_missing_preimage_errors() {
	// §5.4 phase 3: rejected while the new service code is not provided.
	let storage =
		fresh_storage(|s| seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", b"ah-code", RICH));
	let msg = UpwardMessage::UpgradeService {
		code_hash: [0xEE; 32],
		len: 1000.into(),
		min_acc_gas: 100,
		min_memo_gas: 100,
	};
	let digest = ok_digest(ASSET_HUB_PARA_ID, b"ah-code", b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(
		accumulate_logs(&storage, ASSET_HUB_PARA_ID),
		vec![AccumulateLog::ServiceUpgradePreimageMissing { code_hash: [0xEE; 32] }]
	);
}

#[test]
fn rejected_candidate_cannot_use_privileged_calls_works() {
	// §5.1: rejection confines the privileged host functions of §4.3. A candidate
	// rejected at the parent-head check never reaches the replay step, so a
	// stale-parent candidate carrying `UpgradeService` cannot swap the service's
	// own code. The control at the end replays the identical message from an
	// accepted candidate and shows it does take effect, so the block is
	// rejection and not some unrelated precondition.
	use jam_types::CodeHash;

	let new_service_code = b"the-new-parachain-service-code".to_vec();
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", b"ah-code", RICH);
		parachain_service_bin::mock::provide_preimage(s, &new_service_code);
	});
	let new_code_hash = jam_std_common::hash_raw(&new_service_code);
	let msg = UpwardMessage::UpgradeService {
		code_hash: new_code_hash,
		len: (new_service_code.len() as u32).into(),
		min_acc_gas: 100,
		min_memo_gas: 100,
	};
	let original = storage.service(SVC).expect("service exists").code_hash;

	// A REJECTED candidate (stale parent) carrying the upgrade: nothing happens.
	let rejected =
		ok_digest(ASSET_HUB_PARA_ID, b"ah-code", b"not-the-parent", b"ah-1", vec![msg.clone()], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&rejected)], NOW);
	assert_eq!(storage.service(SVC).expect("service exists").code_hash, original);

	// Control: the same message from an accepted candidate does upgrade.
	let accepted =
		ok_digest(ASSET_HUB_PARA_ID, b"ah-code", b"ah-genesis", b"ah-1", vec![msg], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&accepted)], NOW + 1);
	assert_eq!(storage.service(SVC).expect("service exists").code_hash, CodeHash(new_code_hash));
}

#[test]
fn service_upgrade_works() {
	// §5.4: with the preimage present, the upgrade is forwarded to JAM.
	let new_service_code = b"the-new-parachain-service-code".to_vec();
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", b"ah-code", RICH);
		parachain_service_bin::mock::provide_preimage(s, &new_service_code);
	});
	let code_hash = jam_std_common::hash_raw(&new_service_code);
	let msg = UpwardMessage::UpgradeService {
		code_hash,
		len: (new_service_code.len() as u32).into(),
		min_acc_gas: 100,
		min_memo_gas: 100,
	};
	let digest = ok_digest(ASSET_HUB_PARA_ID, b"ah-code", b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert!(accumulate_logs(&storage, ASSET_HUB_PARA_ID).is_empty());
}
