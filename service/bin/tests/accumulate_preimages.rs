//! Preimage solicit/forget lifecycle via upward messages (§6.1) and the
//! `pinned` bit on a para's own validation code (§5.2).

mod common;

use common::*;
use parachain_service::{
	state::log::{AccumulateLog, InsufficientBalanceReason, LogEntry},
	state_balance::preimage_footprint,
};
use parachain_service_interface::{
	types::{Hash, ParaId},
	upward_message::UpwardMessage,
};

const NOW: u32 = 100;
const PARA: ParaId = ParaId(1000);
const CODE: &[u8] = b"para-1000-code";
const BLOB: &[u8] = b"some-arbitrary-preimage-blob";

fn blob_hash() -> Hash {
	jam_std_common::hash_raw(BLOB)
}

fn blob_len() -> u32 {
	BLOB.len() as u32
}

fn accumulate_logs(storage: &jam_node::vm::Storage, para: ParaId) -> Vec<AccumulateLog> {
	para_log(storage, para)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect()
}

/// Storage with `PARA` seeded and one block run that solicits `BLOB`.
fn solicited_storage() -> jam_node::vm::Storage {
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let msg = UpwardMessage::Solicit { hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	storage
}

#[test]
fn solicit_works() {
	// §6.1: an arbitrary preimage is registered with the para as referencer and
	// its footprint charged.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;

	let storage = solicited_storage();

	let entry = registry_entry(&storage, code_ref(BLOB)).expect("registry entry created");
	assert!(entry.referencers.contains(&PARA));
	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.used_state_balance, used_before + preimage_footprint(blob_len()));
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn solicit_insufficient_balance_errors() {
	// §6.1 write-time invariant: no headroom, no registration.
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
	let msg = UpwardMessage::Solicit { hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);

	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	assert!(registry_entry(&storage, code_ref(BLOB)).is_none());
	assert!(matches!(
		accumulate_logs(&storage, PARA)[..],
		[AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::Solicit { .. }
		}]
	));
}

#[test]
fn forget_unprovided_works() {
	// §6.1: a never-provided preimage drops in a single forget, refunding fully.
	let storage = solicited_storage();
	let used_after_solicit = para_info(&storage, PARA).unwrap().used_state_balance;

	let msg = UpwardMessage::Forget { para_id: PARA, hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	assert!(registry_entry(&storage, code_ref(BLOB)).is_none());
	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(
		info.used_state_balance,
		used_after_solicit - preimage_footprint(blob_len()),
		"full refund"
	);
	assert!(accumulate_logs(&storage, PARA).is_empty(), "one-step drop logs nothing");
}

#[test]
fn forget_provided_works() {
	// §6.1 two-step forget: the first forget of a provided preimage only
	// unrequests — referencer retained, still charged, follow-up logged. The
	// second forget past `due` expunges and refunds.
	let mut storage = solicited_storage();
	storage.provide(NOW, SVC, BLOB).expect("solicited in the previous block");
	storage.commit();
	let used_charged = para_info(&storage, PARA).unwrap().used_state_balance;

	let msg = UpwardMessage::Forget { para_id: PARA, hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let logs = accumulate_logs(&storage, PARA);
	let [AccumulateLog::ForgetAgainAt { due, .. }] = logs[..] else {
		panic!("expected ForgetAgainAt, got {logs:?}")
	};
	assert!(registry_entry(&storage, code_ref(BLOB)).is_some(), "referencer retained");
	assert_eq!(para_info(&storage, PARA).unwrap().used_state_balance, used_charged);

	// Second forget, strictly past the turnaround: expunged and refunded.
	let msg = UpwardMessage::Forget { para_id: PARA, hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], due + 2);

	assert!(registry_entry(&storage, code_ref(BLOB)).is_none());
	assert_eq!(
		para_info(&storage, PARA).unwrap().used_state_balance,
		used_charged - preimage_footprint(blob_len())
	);
}

#[test]
fn forget_before_due_works() {
	// §6.1: a second forget before `due` changes nothing and re-logs the due slot.
	let mut storage = solicited_storage();
	storage.provide(NOW, SVC, BLOB).expect("solicited in the previous block");
	storage.commit();

	let msg = UpwardMessage::Forget { para_id: PARA, hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	// Too early: the retry must be rejected without state change.
	let msg = UpwardMessage::Forget { para_id: PARA, hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);

	let logs = accumulate_logs(&storage, PARA);
	assert!(
		matches!(
			logs[..],
			[AccumulateLog::ForgetAgainAt { .. }, AccumulateLog::ForgetAgainAt { .. }]
		),
		"both forgets logged a due slot, got {logs:?}"
	);
	assert!(registry_entry(&storage, code_ref(BLOB)).is_some(), "entry unchanged");
}

#[test]
fn shared_referencer_leaves_works() {
	// §6.1: a non-last referencer leaves immediately — refunded, no JAM forget,
	// the other referencer keeps the preimage live.
	const OTHER: ParaId = ParaId(2000);
	let storage = fresh_storage(|s| {
		seed_para(s, PARA, b"genesis", CODE, RICH);
		seed_para(s, OTHER, b"genesis-2", b"para-2000-code", RICH);
	});

	// Both paras solicit the same blob.
	let msg = UpwardMessage::Solicit { hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg.clone()], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let digest = ok_digest(OTHER, b"para-2000-code", b"genesis-2", b"head-2-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let used_charged = para_info(&storage, PARA).unwrap().used_state_balance;

	// PARA leaves; OTHER remains.
	let msg = UpwardMessage::Forget { para_id: PARA, hash: blob_hash(), len: blob_len().into() };
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let entry = registry_entry(&storage, code_ref(BLOB)).expect("entry stays for OTHER");
	assert!(!entry.referencers.contains(&PARA));
	assert!(entry.referencers.contains(&OTHER));
	assert_eq!(
		para_info(&storage, PARA).unwrap().used_state_balance,
		used_charged - preimage_footprint(blob_len())
	);
	assert!(accumulate_logs(&storage, PARA).is_empty(), "immediate leave logs nothing");
}

#[test]
fn solicit_active_code_pins_works() {
	// §5.2: soliciting the para's own active code only sets `pinned` — the code
	// is already referenced, so no extra balance is charged.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;
	let cref = code_ref(CODE);

	let msg = UpwardMessage::Solicit { hash: cref.hash.0, len: cref.len.into() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, PARA).unwrap();
	assert!(info.validation_code.as_ref().unwrap().pinned);
	assert_eq!(info.used_state_balance, used_before, "no double charge");
}

#[test]
fn forget_active_code_unpins_works() {
	// §5.2: forgetting the own active code only clears `pinned` — the service
	// still needs the code, so the referencer stays and nothing is refunded.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let cref = code_ref(CODE);

	let pin = UpwardMessage::Solicit { hash: cref.hash.0, len: cref.len.into() };
	let unpin = UpwardMessage::Forget { para_id: PARA, hash: cref.hash.0, len: cref.len.into() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![pin, unpin], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, PARA).unwrap();
	assert!(!info.validation_code.as_ref().unwrap().pinned);
	assert!(registry_entry(&storage, cref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn pinned_code_survives_upgrade_works() {
	// §5.2: a pinned old code is NOT released when an upgrade activates — the
	// para keeps (and keeps paying for) its reference.
	const NEW_CODE: &[u8] = b"para-1000-code-v2";
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let old_ref = code_ref(CODE);
	let new_ref = code_ref(NEW_CODE);

	// Pin the active code and request the upgrade in one candidate.
	let pin = UpwardMessage::Solicit { hash: old_ref.hash.0, len: old_ref.len.into() };
	let request = UpwardMessage::RequestCodeUpgrade { hash: new_ref.hash, len: new_ref.len.into() };
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![pin, request], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let used_both = para_info(&storage, PARA).unwrap().used_state_balance;

	// First candidate validated with the new code activates it.
	let digest = ok_digest(PARA, NEW_CODE, b"head-1", b"head-2", vec![], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.validation_code.as_ref().unwrap().code_ref, new_ref);
	// Unlike the unpinned case (see accumulate_upgrades::activation_works), the
	// old code is neither released nor two-step-forgotten.
	assert!(registry_entry(&storage, old_ref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(info.used_state_balance, used_both, "old code still paid for");
	assert!(accumulate_logs(&storage, PARA).is_empty());
}
