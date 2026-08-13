//! Chunked validator-key staging and JAM `designate` (§5.3).

mod common;

use common::*;
use parachain_service::{
	constants::MAX_STAGED_VALIDATOR_KEYS,
	state::{
		log::{AccumulateLog, LogEntry},
		storage_key,
		validator_keys::StagedKeys,
		Tag,
	},
};
use parachain_service_interface::{
	types::{ValidatorKey, ASSET_HUB_PARA_ID},
	upward_message::UpwardMessage,
};

const NOW: u32 = 100;
const AH_CODE: &[u8] = b"ah-code";

fn ah_storage() -> jam_node::vm::Storage {
	fresh_storage(|s| seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH))
}

fn keys(n: usize, fill: u8) -> Vec<ValidatorKey> {
	vec![[fill; 336]; n]
}

fn staged(storage: &jam_node::vm::Storage) -> StagedKeys {
	get_state(storage, &storage_key(Tag::StagedValidatorKeys, &())).unwrap_or_default()
}

fn ah_accumulate_logs(storage: &jam_node::vm::Storage) -> Vec<AccumulateLog> {
	para_log(storage, ASSET_HUB_PARA_ID)
		.into_iter()
		.flat_map(|(_, e)| match e {
			LogEntry::Accumulate { entries } => entries,
			LogEntry::Refine { .. } => panic!("unexpected refine entry"),
		})
		.collect()
}

#[test]
fn chunk_staging_works() {
	let msg = UpwardMessage::SetValidatorKeys { keys: keys(30, 1), is_last: false };
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(ah_storage(), vec![work_item(&digest)], NOW);

	assert_eq!(staged(&storage).len(), 30);
	assert!(mutations.keys.is_none(), "not designated until is_last");
}

#[test]
fn designate_works() {
	// Stage 1000 keys, then finalize with the last 23 — 1023 keys is the full
	// protocol validator count.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		let full: StagedKeys = keys(1000, 1).try_into().unwrap();
		set_state(s, &storage_key(Tag::StagedValidatorKeys, &()), &full);
	});
	let msg = UpwardMessage::SetValidatorKeys { keys: keys(23, 2), is_last: true };
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(storage, vec![work_item(&digest)], NOW);

	assert!(mutations.keys.is_some(), "JAM designate fired");
	assert!(staged(&storage).is_empty(), "staging buffer cleared");
	assert!(ah_accumulate_logs(&storage).is_empty());
}

#[test]
fn designate_wrong_len_errors() {
	// A 5-key set is not the protocol's validator count: rejected, buffer
	// cleared — this doubles as Asset Hub's abort path.
	let msg = UpwardMessage::SetValidatorKeys { keys: keys(5, 1), is_last: true };
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, mutations) = run_block(ah_storage(), vec![work_item(&digest)], NOW);

	assert!(mutations.keys.is_none());
	assert!(staged(&storage).is_empty());
	assert!(matches!(ah_accumulate_logs(&storage)[..], [AccumulateLog::DesignateRejected { .. }]));
}

#[test]
fn staging_overflow_errors() {
	// An append that would exceed the reserved capacity is rejected whole.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		let full: StagedKeys = keys(MAX_STAGED_VALIDATOR_KEYS, 1).try_into().unwrap();
		set_state(s, &storage_key(Tag::StagedValidatorKeys, &()), &full);
	});
	let msg = UpwardMessage::SetValidatorKeys { keys: keys(1, 2), is_last: false };
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", vec![msg], 0);

	let (_, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	assert_eq!(staged(&storage).len(), MAX_STAGED_VALIDATOR_KEYS, "buffer unchanged");
	assert!(matches!(
		ah_accumulate_logs(&storage)[..],
		[AccumulateLog::StagedValidatorKeysOverflow]
	));
}
