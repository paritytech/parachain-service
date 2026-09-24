//! Preimage solicit/forget lifecycle via upward messages (§6.1), including the
//! §5.2 refusal to forget a para's active or announced validation code.

mod common;

use common::*;
use parachain_service::{
	state::log::{AccumulateLog, InsufficientBalanceReason, LogEntry},
	state_balance::preimage_footprint,
};
use parachain_service_core::{
	types::{Hash, ParaId},
	upward_message::{CodeUpgradePhase, Target, UpwardMessage},
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
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
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
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
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

	let msg = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
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

	let msg = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let logs = accumulate_logs(&storage, PARA);
	let [AccumulateLog::ForgetAgainAt { due, .. }] = logs[..] else {
		panic!("expected ForgetAgainAt, got {logs:?}")
	};
	assert!(registry_entry(&storage, code_ref(BLOB)).is_some(), "referencer retained");
	assert_eq!(para_info(&storage, PARA).unwrap().used_state_balance, used_charged);

	// Second forget, strictly past the turnaround: expunged and refunded.
	let msg = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
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

	let msg = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	// Too early: the retry must be rejected without state change.
	let msg = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
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
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg.clone()], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(OTHER),
		hash: blob_hash(),
		len: blob_len().into(),
	};
	let digest = ok_digest(OTHER, b"para-2000-code", b"genesis-2", b"head-2-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let used_charged = para_info(&storage, PARA).unwrap().used_state_balance;

	// PARA leaves; OTHER remains.
	let msg = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: blob_hash(),
		len: blob_len().into(),
	};
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
fn solicit_active_code_is_noop_works() {
	// §5.2: soliciting the para's own active code is already referenced, so it
	// changes nothing and costs no extra state balance.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;
	let cref = code_ref(CODE);

	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: cref.hash.0,
		len: cref.len.into(),
	};
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.validation_code, Some(cref));
	assert_eq!(info.used_state_balance, used_before, "no extra charge");
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn solicit_announced_code_is_noop_works() {
	// §5.2: an announced code is one the para already references, so soliciting
	// it again neither re-charges nor creates a second reference.
	const NEW_CODE: &[u8] = b"para-1000-code-v2";
	let new_ref = code_ref(NEW_CODE);
	let solicit = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: new_ref.hash.0,
		len: new_ref.len.into(),
	};
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![solicit.clone()], 0);
	let (_, mut storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	storage.provide(NOW, SVC, NEW_CODE).expect("solicited in the same block");
	storage.commit();
	let announce = UpwardMessage::RequestCodeUpgrade {
		hash: new_ref.hash,
		len: new_ref.len.into(),
		phase: CodeUpgradePhase::Announcement,
	};
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);
	let announced = para_info(&storage, PARA).unwrap();
	assert_eq!(announced.announced_upgrade, Some(new_ref));

	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![solicit], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);

	let entry = registry_entry(&storage, new_ref).expect("the announced code stays registered");
	assert_eq!(entry.referencers.into_iter().collect::<Vec<_>>(), vec![PARA]);
	assert_eq!(para_info(&storage, PARA).unwrap().used_state_balance, announced.used_state_balance);
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn forget_active_code_refused_works() {
	// §5.2: the running code cannot be forgotten — the referencer and the charge
	// stay, and the refusal is logged.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let cref = code_ref(CODE);
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;

	let forget = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: cref.hash.0,
		len: cref.len.into(),
	};
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![forget], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.used_state_balance, used_before);
	assert!(registry_entry(&storage, cref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::CanNotForgetValidationCode { hash: cref.hash.0, len: cref.len.into() }]
	);
}

#[test]
fn forget_announced_code_refused_works() {
	// §5.2: an announced code is validation code until superseded or applied, so
	// a forget is refused and the reference, charge and announcement all stay.
	const NEW_CODE: &[u8] = b"para-1000-code-v2";
	let new_ref = code_ref(NEW_CODE);
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: new_ref.hash.0,
		len: new_ref.len.into(),
	};
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);
	let (_, mut storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	storage.provide(NOW, SVC, NEW_CODE).expect("solicited in the same block");
	storage.commit();

	let announce = UpwardMessage::RequestCodeUpgrade {
		hash: new_ref.hash,
		len: new_ref.len.into(),
		phase: CodeUpgradePhase::Announcement,
	};
	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);
	let used_announced = para_info(&storage, PARA).unwrap().used_state_balance;

	let forget = UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: new_ref.hash.0,
		len: new_ref.len.into(),
	};
	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![forget], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, Some(new_ref));
	assert_eq!(info.used_state_balance, used_announced);
	assert!(registry_entry(&storage, new_ref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::CanNotForgetValidationCode {
			hash: new_ref.hash.0,
			len: new_ref.len.into()
		}]
	);
}

#[test]
fn forget_running_service_code_works() {
	use parachain_service::state::{preimage_registry::PreimageEntry, storage_key, Tag};
	use parachain_service_core::types::{ASSET_HUB_PARA_ID, CORETIME_PARA_ID};

	// Both Asset Hub itself and Coretime acting on its behalf must preserve the code.
	for origin in [ASSET_HUB_PARA_ID, CORETIME_PARA_ID] {
		let blob = parachain_service_bin::blob();
		let own = code_ref(&blob);
		let other = code_ref(BLOB);
		let storage = fresh_storage(|s| {
			seed_para(s, ASSET_HUB_PARA_ID, b"genesis", CODE, RICH);
			if origin != ASSET_HUB_PARA_ID {
				seed_para(s, origin, b"genesis", b"coretime-code", RICH);
			}
			for reference in [own, other] {
				set_state(
					s,
					&storage_key(Tag::PreimageRegistry, &(reference.hash.0, reference.len)),
					&PreimageEntry { referencers: [ASSET_HUB_PARA_ID].into_iter().collect() },
				);
			}
			s.solicit(0, SVC, other.hash.0, other.len).unwrap();
			let mut info = para_info(s, ASSET_HUB_PARA_ID).unwrap();
			info.used_state_balance += preimage_footprint(own.len) + preimage_footprint(other.len);
			set_state(s, &storage_key(Tag::Parachains, &ASSET_HUB_PARA_ID), &info);
		});
		let before = para_info(&storage, ASSET_HUB_PARA_ID).unwrap().used_state_balance;
		let digest = ok_digest(
			origin,
			if origin == ASSET_HUB_PARA_ID { CODE } else { b"coretime-code" },
			b"genesis",
			b"head-1",
			vec![
				UpwardMessage::Forget {
					target: Target::Parachain(ASSET_HUB_PARA_ID),
					hash: own.hash.0,
					len: own.len.into(),
				},
				UpwardMessage::Forget {
					target: Target::Parachain(ASSET_HUB_PARA_ID),
					hash: other.hash.0,
					len: other.len.into(),
				},
			],
			0,
		);
		let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
		assert!(registry_entry(&storage, own).unwrap().referencers.contains(&ASSET_HUB_PARA_ID));
		assert!(registry_entry(&storage, other).is_none());
		assert_eq!(
			para_info(&storage, ASSET_HUB_PARA_ID).unwrap().used_state_balance,
			before - preimage_footprint(other.len)
		);
		assert!(accumulate_logs(&storage, origin).is_empty());
	}
}
