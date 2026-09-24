//! Scheduled JAM `assign`s: caching, inline application, and recurring queue
//! rotation (§5.1, §7.1, D-7).

mod common;

use common::*;
use executor::pj;
use jam_std_common::{hash_raw, Privileges};
use jam_types::{AccumulateItem, AuthTrace, CodeHash, FixedVec};
use parachain_service::state::{
	assigns::{PendingAssign, PendingAssignCores},
	log::{AccumulateLog, LogEntry},
	storage_key, Tag,
};
use parachain_service_bin::mock::accumulate_context_with_privileges;

use parachain_service_bin::blob as service;
use parachain_service_core::{
	types::{AuthorizerHash, CoreIndex, CORETIME_PARA_ID},
	upward_message::UpwardMessage,
};

const NOW: u32 = 100;
const CT_CODE: &[u8] = b"coretime-code";
const CORE: CoreIndex = 3;
const HASH_A: AuthorizerHash = [0xAA; 32];
const HASH_B: AuthorizerHash = [0xBB; 32];

fn ct_storage() -> jam_node::vm::Storage {
	fresh_storage(|s| seed_para(s, CORETIME_PARA_ID, b"ct-genesis", CT_CODE, RICH))
}

fn assign_msg(queue: Vec<AuthorizerHash>, jam_slot: u32) -> UpwardMessage {
	UpwardMessage::AssignCore { core: CORE, queue, new_assigner: None, jam_slot }
}

fn pending(storage: &jam_node::vm::Storage) -> Option<PendingAssign> {
	get_state(storage, &storage_key(Tag::PendingAssigns, &CORE))
}

fn dirty_cores(storage: &jam_node::vm::Storage) -> PendingAssignCores {
	get_state(storage, &storage_key(Tag::PendingAssignCores, &())).unwrap_or_default()
}

fn ct_accumulate_logs(storage: &jam_node::vm::Storage) -> Vec<AccumulateLog> {
	para_log(storage, CORETIME_PARA_ID)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect()
}

fn privileges_with_assign(assign: u32) -> Privileges {
	Privileges {
		bless: SVC,
		assign: FixedVec::new(assign),
		designate: SVC,
		register: SVC,
		always_acc: Default::default(),
	}
}

/// accumulate_block with explicit JAM Privileges (Todo 6 fixture).
fn run_block_with_privileges(
	storage: jam_node::vm::Storage,
	items: Vec<AccumulateItem>,
	slot: u32,
	privileges: Privileges,
) -> (executor::pj::AccumulateOutcome, jam_node::vm::Storage, jam_node::vm::StateMutations) {
	let engine = jam_node::vm::Engine::new(Some(jam_node::PvmBackend::Interpreter))
		.expect("interpreter engine should initialize");
	let code_hash = CodeHash(hash_raw(&service()));
	let mut context = accumulate_context_with_privileges(storage, items, slot, privileges);
	let outcome = pj::accumulate(&engine, code_hash, &mut context)
		.expect("accumulate should run to completion (not trap)");
	(outcome, context.storage, context.mutations)
}

#[test]
fn schedule_future_works() {
	// A not-yet-due assign is cached, not forwarded.
	let msg = assign_msg(vec![HASH_A, HASH_B], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, mutations) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);

	assert!(mutations.auths.is_empty());
	assert_eq!(
		pending(&storage),
		Some(PendingAssign { queue: vec![HASH_A, HASH_B], assigner: None })
	);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 10)]);
}

#[test]
fn flush_due_works() {
	// The always-accumulate phase forwards a due assign, cycle-expanding the
	// queue to the protocol's exact length (D-7).
	let msg = assign_msg(vec![HASH_A, HASH_B], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, storage, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);

	// An empty block at the due slot flushes it.
	let (_, storage, mutations) = accumulate_block(storage, vec![], NOW + 10);

	let queue: Vec<_> = mutations.auths.get(&CORE).expect("assign fired").clone().into();
	assert_eq!(queue.len(), jam_types::auth_queue_len());
	for (i, hash) in queue.iter().enumerate() {
		let expected = if i % 2 == 0 { HASH_A } else { HASH_B };
		assert_eq!(hash.0, expected, "cycle-repeat at index {i}");
	}
	assert!(pending(&storage).is_none());
	assert!(dirty_cores(&storage).is_empty());
}

#[test]
fn work_error_flushes_due_assignment_works() {
	let due = NOW + 10;
	let msg = assign_msg(vec![HASH_A, HASH_B], due);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, storage, mutations) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);
	assert!(mutations.auths.is_empty());
	let info = para_info(&storage, CORETIME_PARA_ID).unwrap();
	assert_eq!(info.head_data.as_slice(), b"ct-1");
	let log = para_log(&storage, CORETIME_PARA_ID);

	// Skipped work must neither flush early nor bypass always-accumulate when due.
	let (_, storage, mutations) =
		accumulate_block(storage, vec![work_item_skipped(AuthTrace(vec![0xAA; 300]))], due - 1);
	assert!(mutations.auths.is_empty());
	assert_eq!(
		pending(&storage),
		Some(PendingAssign { queue: vec![HASH_A, HASH_B], assigner: None })
	);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, due)]);
	assert_eq!(para_info(&storage, CORETIME_PARA_ID).unwrap(), info);
	assert_eq!(para_log(&storage, CORETIME_PARA_ID), log);

	let (_, storage, mutations) =
		accumulate_block(storage, vec![work_item_skipped(AuthTrace(vec![0xAA; 300]))], due);
	assert_eq!(mutations.auths.len(), 1);
	let queue: Vec<_> = mutations
		.auths
		.get(&CORE)
		.expect("due assign fired despite WorkErr")
		.clone()
		.into();
	assert_eq!(queue.len(), jam_types::auth_queue_len());
	for (i, hash) in queue.iter().enumerate() {
		assert_eq!(hash.0, [HASH_A, HASH_B][i % 2], "cycle-repeat at index {i}");
	}
	assert!(pending(&storage).is_none());
	assert!(dirty_cores(&storage).is_empty());
	assert_eq!(para_info(&storage, CORETIME_PARA_ID).unwrap(), info);
	assert_eq!(para_log(&storage, CORETIME_PARA_ID), log);
}

#[test]
fn inline_when_due_works() {
	// A jam_slot that is already due applies inline in the same block (§5.1).
	let msg = assign_msg(vec![HASH_A], NOW);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, mutations) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);

	assert!(mutations.auths.contains_key(&CORE));
	assert!(pending(&storage).is_none());
	assert!(dirty_cores(&storage).is_empty());
}

#[test]
fn malformed_queue_is_noop_works() {
	// Refine rejects all three shapes. If a crafted digest reaches Accumulate,
	// the defensive branch leaves an existing cached assignment untouched: an
	// empty queue, an over-long one, and a handoff carrying fewer than 80 hashes.
	let msg = assign_msg(vec![HASH_A], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, cached, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);
	assert!(pending(&cached).is_some());

	let short_handoff = UpwardMessage::AssignCore {
		core: CORE,
		queue: vec![HASH_B],
		new_assigner: Some(7),
		jam_slot: NOW,
	};
	for malformed in [
		assign_msg(vec![], NOW),
		assign_msg(vec![HASH_B; jam_types::auth_queue_len() + 1], NOW),
		short_handoff,
	] {
		let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-1", b"ct-2", vec![malformed], 0);
		let (_, storage, mutations) =
			accumulate_block(cached.clone(), vec![work_item(&digest)], NOW + 1);

		assert!(mutations.auths.is_empty());
		assert_eq!(pending(&storage), Some(PendingAssign { queue: vec![HASH_A], assigner: None }));
		assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 10)]);
		assert!(ct_accumulate_logs(&storage).is_empty());
	}
}

#[test]
fn non_tiling_queue_rotates_works() {
	let queue: Vec<AuthorizerHash> = (0..11).map(|i| [i; 32]).collect();
	let msg = assign_msg(queue.clone(), NOW);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, first_mutations) =
		accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);
	let first: Vec<_> = first_mutations.auths.get(&CORE).expect("first cycle fired").clone().into();
	for (i, hash) in first.iter().enumerate() {
		assert_eq!(hash.0, queue[i % queue.len()], "first cycle index {i}");
	}
	let rotated = queue[3..].iter().chain(&queue[..3]).copied().collect::<Vec<_>>();
	assert_eq!(pending(&storage), Some(PendingAssign { queue: rotated.clone(), assigner: None }));
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 80)]);

	let (_, before, mutations) = accumulate_block(storage, vec![], NOW + 79);
	assert!(mutations.auths.is_empty());
	assert_eq!(pending(&before).unwrap().queue, rotated);

	let (_, storage, second_mutations) = accumulate_block(before, vec![], NOW + 80);
	let second: Vec<_> =
		second_mutations.auths.get(&CORE).expect("second cycle fired").clone().into();
	for (i, hash) in second.iter().enumerate() {
		assert_eq!(hash.0, rotated[i % rotated.len()], "second cycle index {i}");
	}
	let rotated_twice = rotated[3..].iter().chain(&rotated[..3]).copied().collect::<Vec<_>>();
	assert_eq!(pending(&storage).unwrap().queue, rotated_twice);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 160)]);
}

#[test]
fn reschedule_overwrites_works() {
	// A second assign for the same core replaces the cached one.
	let first = assign_msg(vec![HASH_A], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![first], 0);
	let (_, storage, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);

	let second = assign_msg(vec![HASH_B], NOW + 20);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-1", b"ct-2", vec![second], 0);
	let (_, storage, _) = accumulate_block(storage, vec![work_item(&digest)], NOW + 1);

	assert_eq!(pending(&storage), Some(PendingAssign { queue: vec![HASH_B], assigner: None }));
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 20)]);
}

#[test]
fn handed_away_core_inline_errors() {
	// §7.1: a due assign for a core another service now owns is rejected by JAM
	// and logged; nothing is written, and the non-tiling queue is not re-armed.
	let msg = assign_msg(vec![HASH_A, HASH_B, HASH_A], NOW);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, mutations) = run_block_with_privileges(
		ct_storage(),
		vec![work_item(&digest)],
		NOW,
		privileges_with_assign(99),
	);

	assert_eq!(ct_accumulate_logs(&storage), vec![AccumulateLog::CoreNotAssignable { core: CORE }]);
	assert!(pending(&storage).is_none(), "no pending assign cached");
	assert!(dirty_cores(&storage).is_empty(), "no dirty-core entry");
	assert!(mutations.auths.is_empty(), "JAM assign must not fire");
}

#[test]
fn handed_away_core_flush_keeps_entry_works() {
	// §7.1: a rotation left armed on a core that has since been handed away does
	// not fire; the entry is left exactly as it was, not consumed or re-armed.
	let msg = assign_msg(vec![HASH_A, HASH_B], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, armed, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);
	let log = para_log(&armed, CORETIME_PARA_ID);

	let (_, storage, mutations) =
		run_block_with_privileges(armed, vec![], NOW + 10, privileges_with_assign(99));

	assert!(mutations.auths.is_empty());
	assert_eq!(
		pending(&storage),
		Some(PendingAssign { queue: vec![HASH_A, HASH_B], assigner: None })
	);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 10)]);
	assert_eq!(para_log(&storage, CORETIME_PARA_ID), log);
}

#[test]
fn assign_with_correct_privilege_works() {
	// Control for `handed_away_core_inline_errors`: the identical inputs with
	// the correct `assign` privilege reach JAM `assign` and fire — proving the
	// negative test discriminates on privilege, not input shape.
	let msg = assign_msg(vec![HASH_A, HASH_B, HASH_A], NOW);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, mutations) = run_block_with_privileges(
		ct_storage(),
		vec![work_item(&digest)],
		NOW,
		privileges_with_assign(SVC),
	);

	assert!(mutations.auths.contains_key(&CORE), "JAM assign fired");
	// 80 % 3 == 2, so the queue resumes rotated by two, 80 slots later.
	assert_eq!(
		pending(&storage),
		Some(PendingAssign { queue: vec![HASH_A, HASH_A, HASH_B], assigner: None })
	);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 80)]);
	assert!(ct_accumulate_logs(&storage).is_empty());
}
