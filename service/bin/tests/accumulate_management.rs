//! Coretime-chain management flows: registration (§6.2), forced updates (§6.3),
//! state-balance authority (§6.1), and clean-up (§6.4).

mod common;

use common::*;
use parachain_service::{
	state::log::{AccumulateLog, LogEntry},
	state_balance::{baseline_for, preimage_footprint},
};
use parachain_service_interface::{
	types::{ParaId, CORETIME_PARA_ID},
	upward_message::UpwardMessage,
};

const NOW: u32 = 100;
const CT_CODE: &[u8] = b"coretime-code";
const NEW_PARA: ParaId = ParaId(3000);
const NEW_CODE: &[u8] = b"para-3000-code";

/// A Coretime-chain candidate carrying `msgs`, enacting `new_head` on `parent`.
fn coretime_digest(
	parent: &[u8],
	new_head: &[u8],
	msgs: Vec<UpwardMessage>,
) -> parachain_service::work_digest::ParachainWorkDigest {
	ok_digest(CORETIME_PARA_ID, CT_CODE, parent, new_head, msgs, 0)
}

fn coretime_storage() -> jam_node::vm::Storage {
	fresh_storage(|s| seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH))
}

fn coretime_accumulate_logs(storage: &jam_node::vm::Storage) -> Vec<AccumulateLog> {
	para_log(storage, CORETIME_PARA_ID)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect()
}

#[test]
fn registration_works() {
	// §6.2: set_state_balance → set_head → set_validation_code, in order.
	let new_ref = code_ref(NEW_CODE);
	let digest = coretime_digest(
		b"ct-genesis",
		b"ct-1",
		vec![
			UpwardMessage::ParachainSetStateBalance { para_id: NEW_PARA, new_total: RICH.into() },
			UpwardMessage::ParachainSetHead {
				para_id: NEW_PARA,
				new_head: b"para-genesis".to_vec().try_into().unwrap(),
			},
			UpwardMessage::ParachainSetValidationCode {
				para_id: NEW_PARA,
				new_validation_code_hash: new_ref.hash,
				new_validation_code_len: new_ref.len.into(),
			},
		],
	);

	let (_, storage, _) = run_block(coretime_storage(), vec![work_item(&digest)], NOW);

	let info = para_info(&storage, NEW_PARA).expect("registration created the ParaInfo");
	assert_eq!(&info.head_data[..], b"para-genesis");
	assert_eq!(info.validation_code.as_ref().unwrap().code_ref, new_ref);
	assert!(!info.validation_code.as_ref().unwrap().pinned);
	assert_eq!(info.total_state_balance, RICH);
	assert_eq!(info.used_state_balance, baseline_for(NEW_PARA) + preimage_footprint(new_ref.len));
	// The service solicited the code on the para's behalf.
	let entry = registry_entry(&storage, new_ref).expect("registry entry created");
	assert!(entry.referencers.contains(&NEW_PARA));
	assert!(coretime_accumulate_logs(&storage).is_empty());
}

#[test]
fn registration_below_baseline_errors() {
	let digest = coretime_digest(
		b"ct-genesis",
		b"ct-1",
		vec![UpwardMessage::ParachainSetStateBalance { para_id: NEW_PARA, new_total: 10.into() }],
	);

	let (_, storage, _) = run_block(coretime_storage(), vec![work_item(&digest)], NOW);

	assert!(para_info(&storage, NEW_PARA).is_none());
	assert!(matches!(
		coretime_accumulate_logs(&storage)[..],
		[AccumulateLog::StateBalanceUpdateRejected { .. }]
	));
}

#[test]
fn lower_total_than_used_errors() {
	// §6.1: Coretime cannot strand currently-paid-for state.
	let storage = fresh_storage(|s| {
		seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH);
		seed_para(s, NEW_PARA, b"para-genesis", NEW_CODE, RICH);
	});
	let used = para_info(&storage, NEW_PARA).unwrap().used_state_balance;
	let digest = coretime_digest(
		b"ct-genesis",
		b"ct-1",
		vec![UpwardMessage::ParachainSetStateBalance {
			para_id: NEW_PARA,
			new_total: (used - 1).into(),
		}],
	);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(para_info(&storage, NEW_PARA).unwrap().total_state_balance, RICH);
	assert!(matches!(
		coretime_accumulate_logs(&storage)[..],
		[AccumulateLog::StateBalanceUpdateRejected { .. }]
	));
}

#[test]
fn forced_set_head_works() {
	let storage = fresh_storage(|s| {
		seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH);
		seed_para(s, NEW_PARA, b"stuck-head", NEW_CODE, RICH);
	});
	let digest = coretime_digest(
		b"ct-genesis",
		b"ct-1",
		vec![UpwardMessage::ParachainSetHead {
			para_id: NEW_PARA,
			new_head: b"recovered".to_vec().try_into().unwrap(),
		}],
	);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(&para_info(&storage, NEW_PARA).unwrap().head_data[..], b"recovered");
}

#[test]
fn forced_set_validation_code_works() {
	// §6.3: displaces the old code (two-step release) and installs the new one.
	let storage = fresh_storage(|s| {
		seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH);
		seed_para(s, NEW_PARA, b"para-genesis", NEW_CODE, RICH);
	});
	let old_ref = code_ref(NEW_CODE);
	let forced_ref = code_ref(b"forced-code");
	let digest = coretime_digest(
		b"ct-genesis",
		b"ct-1",
		vec![UpwardMessage::ParachainSetValidationCode {
			para_id: NEW_PARA,
			new_validation_code_hash: forced_ref.hash,
			new_validation_code_len: forced_ref.len.into(),
		}],
	);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, NEW_PARA).unwrap();
	assert_eq!(info.validation_code.as_ref().unwrap().code_ref, forced_ref);
	assert!(info.pending_upgrade.is_none());
	// The old code was provided, so its first forget only unrequests: the
	// referencer is retained and a follow-up is logged (§6.1).
	assert!(registry_entry(&storage, old_ref).is_some());
	assert!(matches!(
		coretime_accumulate_logs(&storage)[..],
		[AccumulateLog::ForgetAgainAt { .. }]
	));
}

#[test]
fn cleanup_with_extra_state_errors() {
	// §6.4: a para still holding KV state beyond its baseline is rejected.
	let storage = fresh_storage(|s| {
		seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH);
		seed_para(s, NEW_PARA, b"para-genesis", NEW_CODE, RICH);
		// Bump used beyond the allowed clean-up balance, as a kv_set would.
		let mut info = para_info(s, NEW_PARA).unwrap();
		info.used_state_balance += 100;
		set_state(
			s,
			&parachain_service::state::storage_key(
				parachain_service::state::Tag::Parachains,
				&NEW_PARA,
			),
			&info,
		);
	});
	let digest =
		coretime_digest(b"ct-genesis", b"ct-1", vec![UpwardMessage::ParachainCleanUp(NEW_PARA)]);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert!(para_info(&storage, NEW_PARA).is_some());
	assert!(matches!(coretime_accumulate_logs(&storage)[..], [AccumulateLog::TooMuchStateHeld]));
}

#[test]
fn cleanup_unprovided_code_works() {
	// A never-provided code drops in a single forget: full removal right away.
	let storage = fresh_storage(|s| {
		seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH);
		seed_para_unprovided(s, NEW_PARA, b"para-genesis", NEW_CODE, RICH);
	});
	let digest =
		coretime_digest(b"ct-genesis", b"ct-1", vec![UpwardMessage::ParachainCleanUp(NEW_PARA)]);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert!(para_info(&storage, NEW_PARA).is_none());
	assert!(para_log(&storage, NEW_PARA).is_empty());
	assert!(registry_entry(&storage, code_ref(NEW_CODE)).is_none());
	assert!(coretime_accumulate_logs(&storage).is_empty());
}

#[test]
fn cleanup_two_step_works() {
	// §6.4: a provided code needs the two-step forget; the para deregisters,
	// then a retry strictly past the logged `due` completes the removal.
	let storage = fresh_storage(|s| {
		seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH);
		seed_para(s, NEW_PARA, b"para-genesis", NEW_CODE, RICH);
	});
	let digest =
		coretime_digest(b"ct-genesis", b"ct-1", vec![UpwardMessage::ParachainCleanUp(NEW_PARA)]);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, NEW_PARA).expect("still present, deregistering");
	assert!(info.is_deregistering);
	let logs = coretime_accumulate_logs(&storage);
	let [AccumulateLog::ForgetAgainAt { due, .. }] = logs[..] else {
		panic!("expected ForgetAgainAt, got {logs:?}")
	};

	// Retry strictly after `due` completes the expunge and drops everything.
	let retry = coretime_digest(b"ct-1", b"ct-2", vec![UpwardMessage::ParachainCleanUp(NEW_PARA)]);
	let (_, storage, _) = run_block(storage, vec![work_item(&retry)], due + 2);

	assert!(para_info(&storage, NEW_PARA).is_none());
	assert!(registry_entry(&storage, code_ref(NEW_CODE)).is_none());
}
