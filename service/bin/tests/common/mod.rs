//! Shared fixtures for the accumulate integration tests: genesis-state seeding
//! and typed readers over the service's storage layout (§3.1).

#![allow(dead_code)]

pub mod itf;

use codec::{Decode, Encode};
use executor::pj::{self, AccumulateOutcome};
use jam_node::vm::Storage;
use jam_std_common::hash_raw;
use jam_types::{
	AccumulateItem, AuthTrace, CodeHash, Memo, TransferRecord, WorkItemRecord, WorkOutput,
};
use parachain_service::{
	state::{
		log::ParachainLog,
		para_info::{ParaInfo, ValidationCode},
		preimage_registry::PreimageEntry,
		storage_key,
		transfers::{IncomingTransferBuckets, IncomingTransfers},
		Tag,
	},
	state_balance::{baseline_for, preimage_footprint},
	work_digest::{ParachainWorkDigest, RefineLog, ValidationCodeHash, ValidationCodeRef},
};
use parachain_service_bin::{
	mock::{accumulate_context, provide_preimage, MOCK_SERVICE_ID},
	BLOB as SERVICE,
};
use parachain_service_interface::{
	types::{Balance, BucketId, ParaId, ServiceId, Timeslot},
	upward_message::{TransferOutArgs, UpwardMessage},
};

pub const SVC: ServiceId = MOCK_SERVICE_ID;
/// A generously funded default `total_state_balance`.
pub const RICH: Balance = 10_000_000;

/// A `TransferOut` from this service's own regular balance into `dest`'s regular
/// balance — the only shape the vendored GP 0.7.2 host can execute (§5.1).
pub fn transfer_out_msg(
	dest: ServiceId,
	amount: Balance,
	id: u64,
	deferred: Option<([u8; 128], u64)>,
) -> UpwardMessage {
	UpwardMessage::TransferOut(TransferOutArgs {
		source: None,
		dest,
		amount: amount.into(),
		id: id.into(),
		source_supervisor_balance: false,
		dest_supervisor_balance: false,
		deferred,
	})
}

// --- Runner -----------------------------------------------------------------

/// Run one accumulate block over `storage`; returns the outcome and the
/// post-state storage for follow-up blocks/assertions.
pub fn accumulate_block(
	storage: Storage,
	items: Vec<AccumulateItem>,
	slot: Timeslot,
) -> (AccumulateOutcome, Storage, jam_node::vm::StateMutations) {
	run_block_for(SERVICE, storage, items, slot)
}

/// [`accumulate_block`] for an arbitrary service `blob` (e.g. the mock transfer
/// destination); `storage` must hold the blob (see [`fresh_storage_for`]).
pub fn run_block_for(
	blob: &[u8],
	storage: Storage,
	items: Vec<AccumulateItem>,
	slot: Timeslot,
) -> (AccumulateOutcome, Storage, jam_node::vm::StateMutations) {
	let engine = jam_node::vm::Engine::new(Some(jam_node::PvmBackend::Interpreter))
		.expect("interpreter engine should initialize");
	let code_hash = CodeHash(hash_raw(blob));
	let mut context = accumulate_context(storage, items, slot);
	let outcome = pj::accumulate(&engine, code_hash, &mut context)
		.expect("accumulate should run to completion (not trap)");
	(outcome, context.storage, context.mutations)
}

/// Fresh storage holding the service blob, seeded via `seed`.
pub fn fresh_storage(seed: impl FnOnce(&mut Storage)) -> Storage {
	fresh_storage_for(SERVICE, seed)
}

/// [`fresh_storage`] for an arbitrary service `blob`.
pub fn fresh_storage_for(blob: &[u8], seed: impl FnOnce(&mut Storage)) -> Storage {
	let (_, _, context) =
		parachain_service_bin::mock::accumulate_args_at(blob, Vec::new(), 0, seed);
	context.storage
}

// --- State access -----------------------------------------------------------

pub fn set_state(storage: &mut Storage, key: &[u8], value: &impl Encode) {
	storage.set_service_key(SVC, key, &value.encode());
}

pub fn get_state<V: Decode>(storage: &Storage, key: &[u8]) -> Option<V> {
	storage
		.service_key(SVC, key)
		.map(|raw| V::decode(&mut &raw[..]).expect("state written by the service decodes"))
}

pub fn para_info(storage: &Storage, para: ParaId) -> Option<ParaInfo> {
	get_state(storage, &storage_key(Tag::Parachains, &para))
}

pub fn para_log(storage: &Storage, para: ParaId) -> ParachainLog {
	get_state(storage, &storage_key(Tag::ParachainLog, &para)).unwrap_or_default()
}

pub fn registry_entry(storage: &Storage, code: ValidationCodeRef) -> Option<PreimageEntry> {
	get_state(storage, &storage_key(Tag::PreimageRegistry, &(code.hash.0, code.len)))
}

pub fn kv_value(storage: &Storage, para: ParaId, key: &[u8]) -> Option<Vec<u8>> {
	get_state(storage, &storage_key(Tag::KeyValueStorage, &(para, key)))
}

pub fn transfer_queue(storage: &Storage) -> Option<IncomingTransferBuckets> {
	get_state(storage, &storage_key(Tag::IncomingTransferBuckets, &()))
}

pub fn transfer_bucket(storage: &Storage, id: BucketId) -> Option<IncomingTransfers> {
	get_state(storage, &storage_key(Tag::IncomingTransfers, &id))
}

// --- Seeding ----------------------------------------------------------------

pub fn code_ref(code: &[u8]) -> ValidationCodeRef {
	ValidationCodeRef { hash: ValidationCodeHash(hash_raw(code)), len: code.len() as u32 }
}

/// Register `para` with `head` and an active, provided validation `code`, as a
/// completed §6.2 registration would have left it.
pub fn seed_para(storage: &mut Storage, para: ParaId, head: &[u8], code: &[u8], total: Balance) {
	seed_para_inner(storage, para, head, code, total, true)
}

/// Like [`seed_para`], but the validation code was solicited and never provided
/// (registration waiting for the preimage).
pub fn seed_para_unprovided(
	storage: &mut Storage,
	para: ParaId,
	head: &[u8],
	code: &[u8],
	total: Balance,
) {
	seed_para_inner(storage, para, head, code, total, false)
}

/// Register a foreign JAM service (a transfer destination) with the given
/// `min_memo_gas`.
pub fn seed_service(storage: &mut Storage, id: ServiceId, min_memo_gas: u64) {
	let service = jam_std_common::Service {
		code_hash: CodeHash([id as u8; 32]),
		balance: 1_000_000_000,
		min_item_gas: 100,
		min_memo_gas,
		bytes: 0,
		items: 0,
		deposit_offset: 0,
		creation_slot: 0,
		last_accumulation_slot: 0,
		parent_service: 0,
	};
	storage.set_service(id, &service);
	storage.commit();
}

fn seed_para_inner(
	storage: &mut Storage,
	para: ParaId,
	head: &[u8],
	code: &[u8],
	total: Balance,
	provided: bool,
) {
	let cref = code_ref(code);
	let info = ParaInfo {
		head_data: head.to_vec().try_into().expect("test heads fit 4 KiB"),
		validation_code: Some(ValidationCode { code_ref: cref, pinned: false }),
		pending_upgrade: None,
		total_state_balance: total,
		used_state_balance: baseline_for(para) + preimage_footprint(cref.len),
		is_deregistering: false,
	};
	set_state(storage, &storage_key(Tag::Parachains, &para), &info);
	let entry = PreimageEntry { referencers: [para].into_iter().collect() };
	set_state(storage, &storage_key(Tag::PreimageRegistry, &(cref.hash.0, cref.len)), &entry);
	if provided {
		provide_preimage(storage, code);
	} else {
		storage
			.solicit(0, SVC, cref.hash.0, cref.len)
			.expect("preimage should fit in storage");
		storage.commit();
	}
}

// --- Work items -------------------------------------------------------------

/// A successful digest for `para`, built on `parent_head`, validated with
/// `code`, producing `new_head` and `msgs`.
pub fn ok_digest(
	para: ParaId,
	code: &[u8],
	parent_head: &[u8],
	new_head: &[u8],
	msgs: Vec<UpwardMessage>,
	lookup_anchor: Timeslot,
) -> ParachainWorkDigest {
	ParachainWorkDigest::Ok {
		para_id: para,
		validation_code: code_ref(code),
		parent_head_hash: hash_raw(parent_head),
		head_data: new_head.to_vec().try_into().expect("test heads fit 4 KiB"),
		upward_messages: msgs.try_into().expect("test messages fit the bound"),
		lookup_anchor,
	}
}

pub fn err_digest(para: ParaId, error: RefineLog) -> ParachainWorkDigest {
	ParachainWorkDigest::Err { para_id: para, error }
}

/// Wrap a digest as the work-item operand accumulate receives.
pub fn work_item(digest: &ParachainWorkDigest) -> AccumulateItem {
	AccumulateItem::WorkItem(WorkItemRecord {
		package: Default::default(),
		exports_root: Default::default(),
		authorizer_hash: Default::default(),
		payload: Default::default(),
		gas_limit: 0,
		result: Ok(WorkOutput(digest.encode())),
		auth_output: AuthTrace(vec![0xAA; 300]),
	})
}

/// An incoming JAM transfer operand.
pub fn transfer_item(from: ServiceId, amount: Balance) -> AccumulateItem {
	AccumulateItem::Transfer(TransferRecord {
		source: from,
		destination: SVC,
		amount,
		memo: Memo([7; 128]),
		gas_limit: 1_000_000,
	})
}
