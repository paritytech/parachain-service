//! Scheduled JAM `assign`s: caching, inline application, and recurring queue
//! rewrites (§5.1, §7.1, D-7).

mod common;

use common::*;
use executor::pj;
use jam_std_common::{hash_raw, Privileges};
use jam_types::{AccumulateItem, AuthTrace, CodeHash, FixedVec};
use parachain_service::{
	constants::{DUE_ASSIGN_FLUSH_BASE_GAS, DUE_ASSIGN_GAS},
	state::{
		assigns::{PendingAssign, PendingAssignCores},
		log::{AccumulateLog, LogEntry},
		storage_key, Tag,
	},
};
use parachain_service_bin::mock::accumulate_context_with_privileges;
use std::collections::BTreeSet;

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
	run_block_with_gas(storage, items, slot, privileges, None)
}

/// [`run_block_with_privileges`] with an explicit invocation gas pool.
fn run_block_with_gas(
	storage: jam_node::vm::Storage,
	items: Vec<AccumulateItem>,
	slot: u32,
	privileges: Privileges,
	gas: Option<u64>,
) -> (executor::pj::AccumulateOutcome, jam_node::vm::Storage, jam_node::vm::StateMutations) {
	let engine = jam_node::vm::Engine::new(Some(jam_node::PvmBackend::Interpreter))
		.expect("interpreter engine should initialize");
	let code_hash = CodeHash(hash_raw(&service()));
	let mut context = accumulate_context_with_privileges(storage, items, slot, privileges);
	if let Some(gas) = gas {
		context.gas = gas;
	}
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
fn invalid_core_is_noop_works() {
	// Refine rejects a core at or above the core count (§3.3); if a crafted
	// digest carries one, it is neither applied nor cached, due or not.
	let core = jam_types::core_count();
	for jam_slot in [NOW, NOW + 10] {
		let msg =
			UpwardMessage::AssignCore { core, queue: vec![HASH_A], new_assigner: None, jam_slot };
		let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

		let (_, storage, mutations) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);

		assert!(mutations.auths.is_empty());
		let cached: Option<PendingAssign> =
			get_state(&storage, &storage_key(Tag::PendingAssigns, &core));
		assert_eq!(cached, None, "jam_slot {jam_slot}");
		assert!(dirty_cores(&storage).is_empty(), "jam_slot {jam_slot}");
		assert!(ct_accumulate_logs(&storage).is_empty());
	}
}

/// The 80-hash queue JAM got for `CORE`.
fn written(mutations: &jam_node::vm::StateMutations) -> Vec<AuthorizerHash> {
	let queue: Vec<_> = mutations.auths.get(&CORE).expect("assign fired").clone().into();
	queue.into_iter().map(|hash| hash.0).collect()
}

/// §7.1: JAM reads entry `t % 80` at slot `t`, which must hold hash `t % len` of
/// the set, for the 80 slots from `due` on.
fn assert_covers(queue: &[AuthorizerHash], set: &[AuthorizerHash], due: u32) {
	for slot in due..due + 80 {
		assert_eq!(queue[slot as usize % 80], set[slot as usize % set.len()], "slot {slot}");
	}
}

#[test]
fn non_tiling_queue_follows_slots_works() {
	let queue: Vec<AuthorizerHash> = (0..11).map(|i| [i; 32]).collect();
	let msg = assign_msg(queue.clone(), NOW);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, first) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);
	assert_covers(&written(&first), &queue, NOW);
	// The set is kept as sent and rewritten 80 slots later.
	assert_eq!(pending(&storage), Some(PendingAssign { queue: queue.clone(), assigner: None }));
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 80)]);

	let (_, before, mutations) = accumulate_block(storage, vec![], NOW + 79);
	assert!(mutations.auths.is_empty());

	let (_, storage, second) = accumulate_block(before, vec![], NOW + 80);
	assert_covers(&written(&second), &queue, NOW + 80);
	assert_eq!(pending(&storage).unwrap().queue, queue);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 160)]);
}

#[test]
fn design_example_works() {
	// §7.1: the set `[A, B, C]` due at slot 40 starts with `B` at entry 40, slots 80
	// to 119 read entries 0 to 39, and at slot 120 entry 40 starts with `A`.
	let (a, b, c) = ([0xA; 32], [0xB; 32], [0xC; 32]);
	let msg = assign_msg(vec![a, b, c], 40);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, cached, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], 10);

	let (_, storage, mutations) = accumulate_block(cached, vec![], 40);
	let first = written(&mutations);
	assert_eq!((first[40], first[41], first[42], first[79]), (b, c, a, b));
	assert_eq!((first[0], first[1], first[39]), (c, a, c));
	assert_eq!(pending(&storage), Some(PendingAssign { queue: vec![a, b, c], assigner: None }));
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, 120)]);

	let (_, storage, mutations) = accumulate_block(storage, vec![], 120);
	assert_eq!(written(&mutations)[40], a);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, 200)]);
}

#[test]
fn late_flush_rearms_from_due_works() {
	// §7.1: a rewrite carried over past its due slot is still laid out for, and
	// re-armed from, the slot it was due at.
	let queue = vec![HASH_A, HASH_B, HASH_A];
	let msg = assign_msg(queue.clone(), NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, cached, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);

	let (_, storage, mutations) = accumulate_block(cached, vec![], NOW + 15);

	assert_covers(&written(&mutations), &queue, NOW + 10);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 90)]);
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
fn handed_away_core_flush_errors() {
	// §5.1: a rewrite left armed on a core that has since been handed away is
	// rejected by JAM when due: the entry is dropped from both maps and the
	// rejection lands in the Coretime chain's log.
	let msg = assign_msg(vec![HASH_A, HASH_B, HASH_A], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, armed, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);

	let (_, storage, mutations) =
		run_block_with_privileges(armed, vec![], NOW + 10, privileges_with_assign(99));

	assert!(pending(&storage).is_none(), "the dead entry is dropped");
	assert!(dirty_cores(&storage).is_empty(), "and not re-armed");
	assert!(mutations.auths.is_empty());
	let entry =
		LogEntry::Accumulate { entries: vec![AccumulateLog::CoreNotAssignable { core: CORE }] };
	assert_eq!(para_log(&storage, CORETIME_PARA_ID).last(), Some(&(NOW + 10, entry)));
}

#[test]
fn handed_away_core_inline_clears_cache_errors() {
	// §7.1: an inline assign JAM rejects also drops the entry still cached for
	// the core, so it cannot fire later.
	let cached = assign_msg(vec![HASH_A, HASH_B, HASH_A], NOW + 100);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![cached], 0);
	let (_, armed, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);
	assert!(pending(&armed).is_some());
	let inline = assign_msg(vec![HASH_B], NOW + 1);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-1", b"ct-2", vec![inline], 0);

	let (_, storage, mutations) = run_block_with_privileges(
		armed,
		vec![work_item(&digest)],
		NOW + 1,
		privileges_with_assign(99),
	);

	assert!(pending(&storage).is_none(), "the cached entry is dropped");
	assert!(dirty_cores(&storage).is_empty());
	assert!(mutations.auths.is_empty());
	assert_eq!(ct_accumulate_logs(&storage), vec![AccumulateLog::CoreNotAssignable { core: CORE }]);
}

#[test]
fn handed_away_core_flush_log_order_works() {
	// §5.1: the always-accumulate flush runs before the block's work packages, so
	// its entry precedes a same-block Coretime package's own entry.
	let msg = assign_msg(vec![HASH_A, HASH_B, HASH_A], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, armed, _) = accumulate_block(ct_storage(), vec![work_item(&digest)], NOW);
	let other = UpwardMessage::AssignCore {
		core: CORE + 1,
		queue: vec![HASH_B],
		new_assigner: None,
		jam_slot: NOW + 10,
	};
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-1", b"ct-2", vec![other], 0);

	let (_, storage, _) = run_block_with_privileges(
		armed,
		vec![work_item(&digest)],
		NOW + 10,
		privileges_with_assign(99),
	);

	let rejected = |core| {
		(
			NOW + 10,
			LogEntry::Accumulate { entries: vec![AccumulateLog::CoreNotAssignable { core }] },
		)
	};
	let log = para_log(&storage, CORETIME_PARA_ID);
	assert_eq!(log[log.len().saturating_sub(2)..], [rejected(CORE), rejected(CORE + 1)]);
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
	// 80 % 3 != 0, so the set is kept as sent and rewritten 80 slots later.
	assert_eq!(
		pending(&storage),
		Some(PendingAssign { queue: vec![HASH_A, HASH_B, HASH_A], assigner: None })
	);
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 80)]);
	assert!(ct_accumulate_logs(&storage).is_empty());
}

/// Cache an assign of a one-hash queue for each `(core, jam_slot)`.
fn armed(assigns: &[(CoreIndex, u32)]) -> jam_node::vm::Storage {
	let msgs = assigns
		.iter()
		.map(|&(core, jam_slot)| UpwardMessage::AssignCore {
			core,
			queue: vec![HASH_A],
			new_assigner: None,
			jam_slot,
		})
		.collect();
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", msgs, 0);
	accumulate_block(ct_storage(), vec![work_item(&digest)], NOW).1
}

/// A pool whose always-accumulate allowance pays for exactly `n` due assigns
/// on top of `declared`.
fn pool_for(n: u64, declared: u64) -> u64 {
	declared + DUE_ASSIGN_FLUSH_BASE_GAS + (n + 1) * DUE_ASSIGN_GAS - 1
}

#[test]
fn due_assigns_beyond_allowance_carried_over_works() {
	// §5.1: each flush applies only the due assigns its gas pays for, earliest
	// due timeslot first and then lowest core. The allowance pays for one, and
	// what is left of it at the end of the invocation for one more; the rest is
	// carried over to the next block.
	let storage = armed(&[(3, NOW + 8), (4, NOW + 5), (5, NOW + 2), (6, NOW + 5)]);

	let (_, storage, mutations) = run_block_with_gas(
		storage,
		vec![],
		NOW + 10,
		privileges_with_assign(SVC),
		Some(pool_for(1, 0)),
	);

	assert_eq!(mutations.auths.keys().copied().collect::<BTreeSet<_>>(), BTreeSet::from([4, 5]));
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(3, NOW + 8), (6, NOW + 5)]);

	let (_, storage, mutations) =
		run_block_with_privileges(storage, vec![], NOW + 11, privileges_with_assign(SVC));

	assert_eq!(mutations.auths.keys().copied().collect::<BTreeSet<_>>(), BTreeSet::from([3, 6]));
	assert!(dirty_cores(&storage).is_empty());
}

#[test]
fn due_assigns_applied_after_packages_works() {
	// §5.1: due assigns the allowance cannot pay for are applied at the end of
	// the invocation from the gas the work packages left, so a rejection is
	// logged after the same block's Coretime package.
	let storage = armed(&[(CORE, NOW + 10)]);
	let other = UpwardMessage::AssignCore {
		core: CORE + 1,
		queue: vec![HASH_B],
		new_assigner: None,
		jam_slot: NOW + 10,
	};
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-1", b"ct-2", vec![other], 0);

	let (_, storage, _) = run_block_with_gas(
		storage,
		vec![work_item(&digest)],
		NOW + 10,
		privileges_with_assign(99),
		Some(pool_for(0, REPORT_GAS)),
	);

	let rejected = |core| {
		(
			NOW + 10,
			LogEntry::Accumulate { entries: vec![AccumulateLog::CoreNotAssignable { core }] },
		)
	};
	let log = para_log(&storage, CORETIME_PARA_ID);
	assert_eq!(log[log.len().saturating_sub(2)..], [rejected(CORE + 1), rejected(CORE)]);
	assert!(dirty_cores(&storage).is_empty());
}
