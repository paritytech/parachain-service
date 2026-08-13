//! Core per-work-package pipeline tests (§5.1 steps 1–7).

mod common;

use common::*;
use parachain_service::{
	state::log::{AccumulateLog, LogEntry},
	work_digest::RefineLog,
};
use parachain_service_interface::types::ParaId;

const NOW: u32 = 100;
const PARA: ParaId = ParaId(1000);
const CODE: &[u8] = b"para-1000-code";

#[test]
fn enact_works() {
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![], 0);

	let (outcome, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert!(outcome.gas_used > 0);
	let info = para_info(&storage, PARA).expect("para stays registered");
	assert_eq!(&info.head_data[..], b"head-1");
	assert!(para_log(&storage, PARA).is_empty(), "clean enactment logs nothing");
}

#[test]
fn unregistered_para_works() {
	// §5.1 step 1: silently dropped — no state, no log, no panic.
	let storage = fresh_storage(|_| {});
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert!(para_info(&storage, PARA).is_none());
	assert!(para_log(&storage, PARA).is_empty());
}

#[test]
fn deregistering_para_works() {
	// §6.4: a deregistering para is treated as if it no longer exists.
	let storage = fresh_storage(|s| {
		seed_para(s, PARA, b"genesis", CODE, RICH);
		let mut info = para_info(s, PARA).unwrap();
		info.is_deregistering = true;
		set_state(
			s,
			&parachain_service::state::storage_key(
				parachain_service::state::Tag::Parachains,
				&PARA,
			),
			&info,
		);
	});
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(&para_info(&storage, PARA).unwrap().head_data[..], b"genesis");
}

#[test]
fn parent_head_mismatch_works() {
	// §5.1 step 3: rejected silently — no head change, no log entry.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let digest = ok_digest(PARA, CODE, b"not-the-parent", b"head-1", vec![], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(&para_info(&storage, PARA).unwrap().head_data[..], b"genesis");
	assert!(para_log(&storage, PARA).is_empty());
}

#[test]
fn wrong_code_errors() {
	// §5.1 step 5: the authoritative validation-code check logs InvalidCodeHash.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let digest = ok_digest(PARA, b"some-other-code", b"genesis", b"head-1", vec![], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(&para_info(&storage, PARA).unwrap().head_data[..], b"genesis");
	let log = para_log(&storage, PARA);
	assert_eq!(log.len(), 1);
	let (slot, LogEntry::Accumulate { entries }) = &log[0] else {
		panic!("expected an accumulate entry, got {log:?}")
	};
	assert_eq!(*slot, NOW);
	assert_eq!(
		entries,
		&vec![AccumulateLog::InvalidCodeHash { hash: code_ref(b"some-other-code").hash }]
	);
}

#[test]
fn refine_error_logged_works() {
	// §3.3: a Refine failure is logged with the truncated auth trace.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let digest = err_digest(PARA, RefineLog::InvalidCodeHash);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	let log = para_log(&storage, PARA);
	assert_eq!(log.len(), 1);
	let (slot, LogEntry::Refine { error, auth_trace }) = &log[0] else {
		panic!("expected a refine entry, got {log:?}")
	};
	assert_eq!(*slot, NOW);
	assert_eq!(*error, RefineLog::InvalidCodeHash);
	// The 300-byte trace from `work_item` is truncated to the 256-byte cap.
	assert_eq!(auth_trace.len(), 256);
}

#[test]
fn log_pruning_works() {
	// §5.1: entries below a later candidate's lookup-anchor are pruned.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));

	// Block 1: a refine failure lands in the log at slot NOW.
	let bad = err_digest(PARA, RefineLog::InvalidCodeHash);
	let (_, storage, _) = run_block(storage, vec![work_item(&bad)], NOW);
	assert_eq!(para_log(&storage, PARA).len(), 1);

	// Block 2: an enacting candidate whose lookup-anchor is past that entry.
	let good = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![], NOW + 1);
	let (_, storage, _) = run_block(storage, vec![work_item(&good)], NOW + 50);

	assert!(para_log(&storage, PARA).is_empty(), "stale entries pruned");
	assert_eq!(&para_info(&storage, PARA).unwrap().head_data[..], b"head-1");
}

#[test]
fn two_packages_sequence_works() {
	// Two candidates for the same para in one block: the second builds on the
	// first's head.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let first = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![], 0);
	let second = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&first), work_item(&second)], NOW);

	assert_eq!(&para_info(&storage, PARA).unwrap().head_data[..], b"head-2");
}

#[test]
fn kv_set_works() {
	use parachain_service::state_balance::kv_entry_footprint;
	use parachain_service_interface::upward_message::UpwardMessage;

	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;
	let msg = UpwardMessage::SetKV { key: b"k".to_vec(), value: b"value".to_vec() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(kv_value(&storage, PARA, b"k"), Some(b"value".to_vec()));
	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.used_state_balance, used_before + kv_entry_footprint(1, 5));
}

#[test]
fn kv_remove_works() {
	use parachain_service_interface::upward_message::UpwardMessage;

	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;

	let set = UpwardMessage::SetKV { key: b"k".to_vec(), value: b"value".to_vec() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![set], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	let remove = UpwardMessage::RemoveKV { para_id: PARA, key: b"k".to_vec() };
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![remove], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW + 1);

	assert_eq!(kv_value(&storage, PARA, b"k"), None);
	// The full footprint was refunded.
	assert_eq!(para_info(&storage, PARA).unwrap().used_state_balance, used_before);
}

#[test]
fn kv_insufficient_balance_errors() {
	use parachain_service::state::log::InsufficientBalanceReason;
	use parachain_service_interface::upward_message::UpwardMessage;

	// Seed with zero headroom: total == used.
	let storage = fresh_storage(|s| {
		seed_para(s, PARA, b"genesis", CODE, RICH);
		let mut info = para_info(s, PARA).unwrap();
		info.total_state_balance = info.used_state_balance;
		set_state(
			s,
			&parachain_service::state::storage_key(
				parachain_service::state::Tag::Parachains,
				&PARA,
			),
			&info,
		);
	});
	let msg = UpwardMessage::SetKV { key: b"k".to_vec(), value: b"value".to_vec() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(kv_value(&storage, PARA, b"k"), None);
	let log = para_log(&storage, PARA);
	let (_, LogEntry::Accumulate { entries }) = &log[0] else { panic!("{log:?}") };
	assert!(matches!(
		entries[0],
		AccumulateLog::InsufficientStateBalance { reason: InsufficientBalanceReason::SetKV { .. } }
	));
	// The candidate itself still enacted — only the write was rejected.
	assert_eq!(&para_info(&storage, PARA).unwrap().head_data[..], b"head-1");
}
