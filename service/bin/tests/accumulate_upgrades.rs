//! Code-upgrade lifecycle (§5.2) and service self-upgrade (§5.4).
//!
//! §5.2 has two phases and no deadline: an `Announcement` arms a code that a
//! later `Apply` activates; the announcement stands until applied or superseded.

mod common;

use common::*;
use parachain_service::{
	state::log::{AccumulateLog, InsufficientBalanceReason, LogEntry},
	state_balance::preimage_footprint,
};
use parachain_service_core::{
	types::{ParaId, ASSET_HUB_PARA_ID},
	upward_message::{CodeUpgradePhase, Target, UpwardMessage},
};

const NOW: u32 = 100;
const PARA: ParaId = ParaId(1000);
const CODE: &[u8] = b"para-1000-code";
const NEW_CODE: &[u8] = b"para-1000-code-v2";
const THIRD_CODE: &[u8] = b"para-1000-code-v3";

fn accumulate_logs(storage: &jam_node::vm::Storage, para: ParaId) -> Vec<AccumulateLog> {
	para_log(storage, para)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect()
}

fn announce_msg(code: &[u8]) -> UpwardMessage {
	let reference = code_ref(code);
	UpwardMessage::RequestCodeUpgrade {
		hash: reference.hash,
		len: reference.len.into(),
		phase: CodeUpgradePhase::Announcement,
	}
}

fn apply_msg(code: &[u8]) -> UpwardMessage {
	let reference = code_ref(code);
	UpwardMessage::RequestCodeUpgrade {
		hash: reference.hash,
		len: reference.len.into(),
		phase: CodeUpgradePhase::Apply,
	}
}

fn forget_msg(code: &[u8]) -> UpwardMessage {
	let reference = code_ref(code);
	UpwardMessage::Forget {
		target: Target::Parachain(PARA),
		hash: reference.hash.0,
		len: reference.len.into(),
	}
}

/// Solicit `code` for `PARA` in its own candidate (validated with the active
/// `CODE`), then provide the blob to JAM so a later `Announcement` sees it as
/// available. `parent`/`next` are the candidate's heads; the digest's lookup
/// anchor is 0, which a provided preimage is available at.
fn solicit_and_provide(
	storage: jam_node::vm::Storage,
	parent: &[u8],
	next: &[u8],
	code: &[u8],
	slot: u32,
) -> jam_node::vm::Storage {
	let reference = code_ref(code);
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: reference.hash.0,
		len: reference.len.into(),
	};
	let digest = ok_digest(PARA, CODE, parent, next, vec![msg], 0);
	let (_, mut storage, _) = accumulate_block(storage, vec![work_item(&digest)], slot);
	storage.provide(slot, SVC, code).expect("solicited in the same block");
	storage.commit();
	storage
}

#[test]
fn announce_works() {
	// §5.2: soliciting the new code references + charges it; announcing it only
	// arms the upgrade, leaving the active code in place.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let used_before = para_info(&storage, PARA).unwrap().used_state_balance;

	let storage = solicit_and_provide(storage, b"genesis", b"head-1", NEW_CODE, NOW);

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, Some(new_ref));
	assert_eq!(info.validation_code, Some(code_ref(CODE)), "the active code is untouched");
	assert_eq!(info.used_state_balance, used_before + preimage_footprint(new_ref.len));
	assert!(registry_entry(&storage, new_ref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn activation_works() {
	// §5.2: the `Apply` swaps the two code slots. The candidate carrying it is
	// still validated with the old active code; the displaced code merely unpins.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let storage = solicit_and_provide(storage, b"genesis", b"head-1", NEW_CODE, NOW);
	let used_charged = para_info(&storage, PARA).unwrap().used_state_balance;

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);
	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![apply_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.validation_code, Some(new_ref));
	assert_eq!(info.announced_upgrade, None);
	assert_eq!(&info.head_data[..], b"head-3");
	// The displaced old code is neither released nor forgotten: it stays
	// referenced and charged until the para forgets it (§5.2).
	assert!(registry_entry(&storage, code_ref(CODE)).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(info.used_state_balance, used_charged, "apply swaps slots, charges nothing");
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn old_code_candidate_keeps_announcement_works() {
	// §5.2 phase 4 + no deadline: a candidate still validated with the old code
	// enacts, and the announcement survives even far past any timeout.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let storage = solicit_and_provide(storage, b"genesis", b"head-1", NEW_CODE, NOW);

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 100_000);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(&info.head_data[..], b"head-3", "the old-code candidate enacted");
	assert_eq!(info.validation_code, Some(code_ref(CODE)));
	assert_eq!(info.announced_upgrade, Some(new_ref), "an announcement never times out");
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn announcement_unavailable_errors() {
	// §5.2: a solicited-but-never-provided code is not available, so it cannot
	// be announced. The failure is logged and nothing changes.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: new_ref.hash.0,
		len: new_ref.len.into(),
	};
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, None);
	assert_eq!(info.validation_code, Some(code_ref(CODE)));
	assert!(registry_entry(&storage, new_ref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::CodeUpgradeNotAvailable {
			hash: new_ref.hash.0,
			len: new_ref.len.into()
		}]
	);
}

#[test]
fn announcement_of_other_paras_code_errors() {
	// §5.2: a code another para paid for cannot be announced, even when it is
	// available — the caller must be a referencer itself.
	const OTHER: ParaId = ParaId(2000);
	let storage = fresh_storage(|s| {
		seed_para(s, PARA, b"genesis", CODE, RICH);
		seed_para(s, OTHER, b"genesis-2", b"para-2000-code", RICH);
	});
	let new_ref = code_ref(NEW_CODE);
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(OTHER),
		hash: new_ref.hash.0,
		len: new_ref.len.into(),
	};
	let digest = ok_digest(OTHER, b"para-2000-code", b"genesis-2", b"head-2-1", vec![msg], 0);
	let (_, mut storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	storage.provide(NOW, SVC, NEW_CODE).expect("solicited by OTHER");
	storage.commit();

	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, None);
	assert_eq!(info.validation_code, Some(code_ref(CODE)));
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::CodeUpgradeNotAvailable {
			hash: new_ref.hash.0,
			len: new_ref.len.into()
		}]
	);
}

#[test]
fn apply_without_announcement_errors() {
	// §5.2: an `Apply` with no standing announcement is refused.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![apply_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.validation_code, Some(code_ref(CODE)));
	assert_eq!(info.announced_upgrade, None);
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::CodeUpgradeNotAnnounced {
			hash: new_ref.hash.0,
			len: new_ref.len.into()
		}]
	);
}

#[test]
fn apply_mismatched_announcement_errors() {
	// §5.2: an `Apply` naming a different code than the standing announcement is
	// refused; the announcement is preserved.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let third_ref = code_ref(THIRD_CODE);
	let storage = solicit_and_provide(storage, b"genesis", b"head-1", NEW_CODE, NOW);

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);
	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![apply_msg(THIRD_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.validation_code, Some(code_ref(CODE)));
	assert_eq!(info.announced_upgrade, Some(new_ref));
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::CodeUpgradeNotAnnounced {
			hash: third_ref.hash.0,
			len: third_ref.len.into()
		}]
	);
}

#[test]
fn announce_active_code_is_noop_works() {
	// §5.2: announcing the running code cannot shadow it, so the act is a no-op.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let before = para_info(&storage, PARA).unwrap();
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", vec![announce_msg(CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.validation_code, before.validation_code);
	assert_eq!(info.announced_upgrade, None, "the active code is never announced");
	assert_eq!(info.used_state_balance, before.used_state_balance);
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn announcement_supersedes_previous_works() {
	// §5.2: a second announcement replaces the first; the superseded code merely
	// unpins — still referenced and charged until the para forgets it.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let third_ref = code_ref(THIRD_CODE);
	let storage = solicit_and_provide(storage, b"genesis", b"head-1", NEW_CODE, NOW);
	let storage = solicit_and_provide(storage, b"head-1", b"head-2", THIRD_CODE, NOW + 1);
	let used_both = para_info(&storage, PARA).unwrap().used_state_balance;

	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);
	let digest = ok_digest(PARA, CODE, b"head-3", b"head-4", vec![announce_msg(THIRD_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 3);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, Some(third_ref));
	assert_eq!(info.validation_code, Some(code_ref(CODE)));
	assert!(registry_entry(&storage, new_ref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(info.used_state_balance, used_both);
	assert!(accumulate_logs(&storage, PARA).is_empty());
}

#[test]
fn forget_announced_refused_then_supersede_works() {
	// §5.2: a `Forget` of the announced code is refused while it is pinned; a
	// superseding announcement unpins it, and only then does the forget take.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let new_ref = code_ref(NEW_CODE);
	let third_ref = code_ref(THIRD_CODE);
	let storage = solicit_and_provide(storage, b"genesis", b"head-1", NEW_CODE, NOW);
	let storage = solicit_and_provide(storage, b"head-1", b"head-2", THIRD_CODE, NOW + 1);

	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);
	let used_announced = para_info(&storage, PARA).unwrap().used_state_balance;

	// Refused while announced: reference, charge and status all stay.
	let digest = ok_digest(PARA, CODE, b"head-3", b"head-4", vec![forget_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 3);
	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, Some(new_ref));
	assert!(registry_entry(&storage, new_ref).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(info.used_state_balance, used_announced);
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::CanNotForgetValidationCode {
			hash: new_ref.hash.0,
			len: new_ref.len.into()
		}]
	);

	// Supersede with THIRD, unpinning NEW; the forget now releases it (two-step,
	// because NEW was provided).
	let digest = ok_digest(PARA, CODE, b"head-4", b"head-5", vec![announce_msg(THIRD_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 4);
	let digest = ok_digest(PARA, CODE, b"head-5", b"head-6", vec![forget_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 5);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, Some(third_ref));
	let logs = accumulate_logs(&storage, PARA);
	assert!(
		matches!(logs.last(), Some(AccumulateLog::ForgetAgainAt { .. })),
		"the post-supersede forget releases NEW, got {logs:?}"
	);
	assert!(
		registry_entry(&storage, new_ref).is_some_and(|e| e.referencers.contains(&PARA)),
		"first forget of a provided code only unrequests"
	);
}

#[test]
fn insufficient_balance_preserves_announcement_works() {
	// §5.2/§6.1: a failed solicit leaves the standing announcement intact, and
	// the unaffordable code then fails to announce as unavailable.
	let new_ref = code_ref(NEW_CODE);
	let third_ref = code_ref(THIRD_CODE);
	let storage = fresh_storage(|s| {
		seed_para(s, PARA, b"genesis", CODE, RICH);
		let mut info = para_info(s, PARA).unwrap();
		// Headroom for exactly one more preimage.
		info.total_state_balance = info.used_state_balance + preimage_footprint(new_ref.len);
		set_state(
			s,
			&parachain_service::state::storage_key(
				parachain_service::state::Tag::Parachains,
				&PARA,
			),
			&info,
		);
	});
	let storage = solicit_and_provide(storage, b"genesis", b"head-1", NEW_CODE, NOW);

	let digest = ok_digest(PARA, CODE, b"head-1", b"head-2", vec![announce_msg(NEW_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	// The second solicit cannot be afforded: rejected and logged.
	let msg = UpwardMessage::Solicit {
		target: Target::Parachain(PARA),
		hash: third_ref.hash.0,
		len: third_ref.len.into(),
	};
	let digest = ok_digest(PARA, CODE, b"head-2", b"head-3", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 2);
	assert!(registry_entry(&storage, third_ref).is_none());
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::Solicit {
				hash: third_ref.hash.0,
				len: third_ref.len.into()
			}
		}]
	);

	// Announcing the unaffordable code is then rejected as unavailable.
	let digest = ok_digest(PARA, CODE, b"head-3", b"head-4", vec![announce_msg(THIRD_CODE)], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 3);

	let info = para_info(&storage, PARA).unwrap();
	assert_eq!(info.announced_upgrade, Some(new_ref), "standing announcement preserved");
	assert_eq!(
		accumulate_logs(&storage, PARA),
		vec![
			AccumulateLog::InsufficientStateBalance {
				reason: InsufficientBalanceReason::Solicit {
					hash: third_ref.hash.0,
					len: third_ref.len.into()
				}
			},
			AccumulateLog::CodeUpgradeNotAvailable {
				hash: third_ref.hash.0,
				len: third_ref.len.into()
			},
		]
	);
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

	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

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
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&rejected)], NOW);
	assert_eq!(storage.service(SVC).expect("service exists").code_hash, original);

	// Control: the same message from an accepted candidate does upgrade.
	let accepted = ok_digest(ASSET_HUB_PARA_ID, b"ah-code", b"ah-genesis", b"ah-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&accepted)], NOW + 1);
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

	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	assert!(accumulate_logs(&storage, ASSET_HUB_PARA_ID).is_empty());
}
