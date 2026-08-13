//! Scheduled JAM `assign`s: caching, inline application, cancellation, and the
//! always-accumulate flush (§5.1, §7.1, D-7).

mod common;

use common::*;
use parachain_service::state::{
	assigns::{PendingAssign, PendingAssignCores},
	storage_key, Tag,
};
use parachain_service_interface::{
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

#[test]
fn schedule_future_works() {
	// A not-yet-due assign is cached, not forwarded.
	let msg = assign_msg(vec![HASH_A, HASH_B], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(ct_storage(), vec![work_item(&digest)], NOW);

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
	let (_, storage, _) = run_block(ct_storage(), vec![work_item(&digest)], NOW);

	// An empty block at the due slot flushes it.
	let (_, storage, mutations) = run_block(storage, vec![], NOW + 10);

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
fn inline_when_due_works() {
	// A jam_slot that is already due applies inline in the same block (§5.1).
	let msg = assign_msg(vec![HASH_A], NOW);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(ct_storage(), vec![work_item(&digest)], NOW);

	assert!(mutations.auths.contains_key(&CORE));
	assert!(pending(&storage).is_none());
	assert!(dirty_cores(&storage).is_empty());
}

#[test]
fn cancel_works() {
	// An empty queue cancels the cached entry without a JAM call.
	let msg = assign_msg(vec![HASH_A], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![msg], 0);
	let (_, storage, _) = run_block(ct_storage(), vec![work_item(&digest)], NOW);
	assert!(pending(&storage).is_some());

	let cancel = assign_msg(vec![], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-1", b"ct-2", vec![cancel], 0);
	let (_, storage, mutations) = run_block(storage, vec![work_item(&digest)], NOW + 1);

	assert!(mutations.auths.is_empty());
	assert!(pending(&storage).is_none());
	assert!(dirty_cores(&storage).is_empty());
}

#[test]
fn reschedule_overwrites_works() {
	// A second assign for the same core replaces the cached one.
	let first = assign_msg(vec![HASH_A], NOW + 10);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-genesis", b"ct-1", vec![first], 0);
	let (_, storage, _) = run_block(ct_storage(), vec![work_item(&digest)], NOW);

	let second = assign_msg(vec![HASH_B], NOW + 20);
	let digest = ok_digest(CORETIME_PARA_ID, CT_CODE, b"ct-1", b"ct-2", vec![second], 0);
	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW + 1);

	assert_eq!(pending(&storage), Some(PendingAssign { queue: vec![HASH_B], assigner: None }));
	assert_eq!(dirty_cores(&storage).to_vec(), vec![(CORE, NOW + 20)]);
}
