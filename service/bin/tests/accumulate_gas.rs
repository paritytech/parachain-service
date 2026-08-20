//! Worst-case accumulate gas measurements, run through the real PVM
//! interpreter with full protocol parameters.
//!
//! Reference points: the Gray Paper caps accumulation of a single work-report
//! at `Ga = max_accumulate_gas = 10_000_000` gas, and the elective data of a
//! work-report (which the digest is part of) at `Wr = 48 KiB`. A §4.3 digest
//! holds at most [`MAX_UPWARD_MESSAGES_PER_DIGEST`] = 1024 messages.
//!
//! Run with `--nocapture` to see the measurements.

mod common;

use codec::Encode;
use common::*;
use jam_types::{AccumulateItem, Memo as JamMemo, TransferRecord};
use parachain_service_bin::MOCK_DEST_BLOB;
use parachain_service::{
	constants::{CORE_COUNT, MAX_TRANSFER_GAS},
	state::{
		assigns::{PendingAssign, PendingAssignCores},
		storage_key, Tag,
	},
	work_digest::{ValidationCodeHash, ValidationCodeRef},
};
use parachain_service_interface::{
	types::{CoreIndex, Hash, ParaId, ASSET_HUB_PARA_ID},
	upward_message::{UpwardMessage, MAX_UPWARD_MESSAGES_PER_DIGEST},
};

const NOW: u32 = 100;
const PARA: ParaId = ParaId(1000);
const CODE: &[u8] = b"para-1000-code";
const AH_CODE: &[u8] = b"ah-code";
const FLOOD: u32 = MAX_UPWARD_MESSAGES_PER_DIGEST;

/// Gray Paper `Wr`: max total size of the unbounded blobs in a work-report.
const MAX_REPORT_ELECTIVE_DATA: usize = 48 * 1024;

fn distinct_hash(i: u32) -> Hash {
	let mut hash = [0u8; 32];
	hash[..4].copy_from_slice(&i.to_le_bytes());
	hash
}

fn report(name: &str, gas: u64, elapsed: std::time::Duration, digest_len: usize) {
	let ga = jam_types::max_accumulate_gas();
	eprintln!(
		"{name}: gas_used={gas} ({:.2}x Ga={ga}), digest={digest_len} B, elapsed={elapsed:?}",
		gas as f64 / ga as f64,
	);
}

/// Pinned measurements of the flood benchmarks below. Gas in the PVM
/// interpreter is deterministic, so each benchmark asserts its measurement
/// still equals its pin — any guest or host change that shifts a worst case
/// must update the pin here, and [`worst_case_margin_works`] then re-checks
/// the margin invariant against the new value.
mod measured {
	/// 1024-solicit digest — the heaviest reachable digest replay.
	pub const SOLICIT_FLOOD: u64 = 7_764_545;
	/// 1024 small KV writes.
	pub const SET_KV_FLOOD: u64 = 5_926_559;
	/// 1024 outbound transfers to a friendly destination (digest exceeds `Wr`).
	pub const TRANSFER_OUT_FLOOD: u64 = 3_013_673;
	/// 331 outbound transfers to a destination demanding the full cap (F-13).
	pub const HOSTILE_DEST_FLOOD: u64 = 33_929_854;
	/// 1024 incoming transfers recorded in one bucket write (D-8).
	pub const INCOMING_TRANSFER_FLOOD: u64 = 1_638_929;
	/// Due `assign` flush for all 341 cores in one block (F-12).
	pub const DUE_ASSIGN_FLOOD: u64 = 9_942_768;
	/// Marginal cost of a realistic destination's memo handler, per transfer.
	pub const DEST_HANDLER_PER_TRANSFER: u64 = 1_665;
	/// Gas for a single real ed25519 is_authorized call (Merkle proof + ed25519
	/// verify_strict) — must stay under Gi/5 per the 20% margin requirement.
	pub const IS_AUTHORIZED_ED25519: u64 = 1_010_744;
}

/// The F-10 invariant, statically: every reachable worst case must leave real
/// headroom below its budget, so host-gas recalibration, fatter state, or code
/// growth surface as a failing margin here — not as a silent OOG cliff in
/// production. The pins are kept honest by the benchmarks in this file.
#[test]
fn worst_case_margin_works() {
	let ga = jam_types::max_accumulate_gas();
	// A reachable worst-case digest replay must leave at least 20% of Ga.
	let budget = ga - ga / 5;
	assert!(measured::SOLICIT_FLOOD <= budget);
	assert!(measured::SET_KV_FLOOD <= budget);
	assert!(measured::INCOMING_TRANSFER_FLOOD <= budget);
	// The one-off due-assign avalanche is paid by the `always_acc` allotment
	// (F-12); this pin is the sizing input — an allotment of Ga covers it.
	assert!(measured::DUE_ASSIGN_FLOOD <= ga);
	// A realistic destination handler fits in a tenth of the transfer cap, so
	// the cap could drop to `Ga / 1000` and still leave ~6x headroom.
	assert!(measured::DEST_HANDLER_PER_TRANSFER <= MAX_TRANSFER_GAS / 10);
	// FIXME: F-13 — the individually-capped hostile-destination digest still
	// exceeds Ga. Flip this to `<= budget` once the replay loop gets a
	// cumulative forwarded-gas budget (or the §4.3 caps are co-derived).
	assert!(measured::HOSTILE_DEST_FLOOD > ga);
}

#[test]
fn solicit_flood_works() {
	// The heaviest realistic flood: 1024 distinct-preimage solicits fit within
	// `Wr`, and each does a registry write + ParaInfo update + JAM `solicit`.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let msgs = (0..FLOOD)
		.map(|i| UpwardMessage::Solicit { hash: distinct_hash(i), len: 100.into() })
		.collect();
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", msgs, 0);
	let digest_len = digest.encode().len();
	assert!(digest_len <= MAX_REPORT_ELECTIVE_DATA, "this flood is reachable in practice");

	let (outcome, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	report("solicit_flood", outcome.gas_used, outcome.elapsed, digest_len);
	// All 1024 solicits applied.
	let last = ValidationCodeRef {
		hash: ValidationCodeHash(distinct_hash(FLOOD - 1)),
		len: 100,
	};
	assert!(registry_entry(&storage, last).is_some_and(|e| e.referencers.contains(&PARA)));
	assert_eq!(&para_info(&storage, PARA).unwrap().head_data[..], b"head-1");
	// A reachable worst-case digest must stay accumulable within Ga, or a valid
	// candidate un-enacts mid-replay (F-10) — `worst_case_margin_works` holds
	// the margin; this pin keeps it honest.
	assert_eq!(outcome.gas_used, measured::SOLICIT_FLOOD);
}

#[test]
fn set_kv_flood_works() {
	// 1024 distinct small KV writes.
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let msgs = (0..FLOOD)
		.map(|i| UpwardMessage::SetKV { key: i.to_le_bytes().to_vec(), value: vec![0xAB; 8] })
		.collect();
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", msgs, 0);
	let digest_len = digest.encode().len();
	assert!(digest_len <= MAX_REPORT_ELECTIVE_DATA, "this flood is reachable in practice");

	let (outcome, storage, _) = run_block(storage, vec![work_item(&digest)], NOW);

	report("set_kv_flood", outcome.gas_used, outcome.elapsed, digest_len);
	let last_key = (FLOOD - 1).to_le_bytes();
	assert_eq!(kv_value(&storage, PARA, &last_key), Some(vec![0xAB; 8]));
	assert_eq!(outcome.gas_used, measured::SET_KV_FLOOD);
}

#[test]
fn transfer_out_flood_works() {
	// 1024 outbound transfers. NOTE: the encoded digest exceeds `Wr`, so this
	// flood cannot occur in a real work-report — the per-type reachable maximum
	// is ~Wr / 148 B ≈ 331 messages. Measured anyway as the §4.3 cap's worst case.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, 500);
	});
	let msgs = (0..FLOOD)
		.map(|i| transfer_out_msg(42, 1, i as u64, Some(([7; 128], 500))))
		.collect();
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", msgs, 0);
	let digest_len = digest.encode().len();
	assert!(digest_len > MAX_REPORT_ELECTIVE_DATA, "unreachable in a real report (F-11)");

	let (outcome, _, mutations) = run_block(storage, vec![work_item(&digest)], NOW);

	report("transfer_out_flood", outcome.gas_used, outcome.elapsed, digest_len);
	assert_eq!(mutations.transfers.len(), FLOOD as usize);
	// This measures the replay machinery alone: the mock destination only
	// demands `min_memo_gas = 500`, while GP `Ω_T` charges each transfer's
	// forwarded gas to the sender's meter — see
	// `transfer_out_hostile_dest_flood_works` for the forwarded-gas worst case.
	assert_eq!(outcome.gas_used, measured::TRANSFER_OUT_FLOOD);
}

/// The largest transfer count whose digest still fits `Wr` (F-11). A deferred
/// `TransferOut` encodes to ~148 B (memo alone is 128 B), so `Wr / 148 B`
/// leaves 331 as the reachable maximum (verified against the encoded digest).
const WR_TRANSFER_FLOOD: u32 = 331;

#[test]
fn transfer_out_hostile_dest_flood_works() {
	// The forwarded-gas worst case (F-13): a `Wr`-sized digest of transfers each
	// forwarding the D-6 per-transfer maximum. GP `Ω_T` charges every forwarded
	// limit to the sender's meter, so this digest costs ~331 x 100k on top of the
	// replay machinery — several times Ga, although each transfer is individually
	// capped. Since §5.1 moved the gas choice to the caller, the worst case is now
	// Asset Hub's to cause rather than a hostile destination's.
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, MAX_TRANSFER_GAS);
	});
	let msgs = (0..WR_TRANSFER_FLOOD)
		.map(|i| transfer_out_msg(42, 1, i as u64, Some(([7; 128], MAX_TRANSFER_GAS))))
		.collect();
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", msgs, 0);
	let digest_len = digest.encode().len();
	assert!(digest_len <= MAX_REPORT_ELECTIVE_DATA, "this flood is reachable in practice");

	let (outcome, _, mutations) = run_block(storage, vec![work_item(&digest)], NOW);

	report("transfer_out_hostile_dest_flood", outcome.gas_used, outcome.elapsed, digest_len);
	assert_eq!(mutations.transfers.len(), WR_TRANSFER_FLOOD as usize);
	// A reachable, individually-capped digest exceeds Ga: in production it
	// would OOG mid-replay and un-enact the validated candidate (F-13).
	assert_eq!(outcome.gas_used, measured::HOSTILE_DEST_FLOOD);
}

/// An incoming transfer for the mock destination service, carrying a distinct
/// deposit reference in the memo.
fn dest_transfer_item(i: u32) -> AccumulateItem {
	let mut memo = [0u8; 128];
	memo[..4].copy_from_slice(&i.to_le_bytes());
	AccumulateItem::Transfer(TransferRecord {
		source: 9 + i,
		destination: SVC,
		amount: 1_000_000,
		memo: JamMemo(memo),
		gas_limit: MAX_TRANSFER_GAS,
	})
}

#[test]
fn dest_handler_flood_works() {
	// What must MAX_TRANSFER_GAS cover on the receiving side? The mock
	// destination's memo handler does the realistic minimum of bookkeeping —
	// forward map (deposit reference -> sender, amount), backward map
	// (sender -> reference), counter increment (read-modify-write per
	// transfer). The empty-block run isolates the per-transfer marginal cost
	// from the invocation overhead.
	let (empty, _, _) =
		run_block_for(MOCK_DEST_BLOB, fresh_storage_for(MOCK_DEST_BLOB, |_| {}), vec![], NOW);
	let items = (0..FLOOD).map(dest_transfer_item).collect();

	let (outcome, storage, _) =
		run_block_for(MOCK_DEST_BLOB, fresh_storage_for(MOCK_DEST_BLOB, |_| {}), items, NOW);

	let per_transfer = (outcome.gas_used - empty.gas_used) / FLOOD as u64;
	report("dest_handler_flood", outcome.gas_used, outcome.elapsed, 0);
	eprintln!("dest_handler_flood: marginal per-transfer gas: {per_transfer}");
	// All transfers handled: counter at FLOOD, both maps hold the last entry.
	let last = FLOOD - 1;
	assert_eq!(
		storage.service_key(SVC, b"c").as_deref(),
		Some(&(FLOOD as u64).to_le_bytes()[..])
	);
	let mut fwd_key = [0u8; 33];
	fwd_key[0] = b'f';
	fwd_key[1..5].copy_from_slice(&last.to_le_bytes());
	let fwd = storage.service_key(SVC, &fwd_key).expect("forward entry written");
	assert_eq!(&fwd[..4], &(9 + last).to_le_bytes()[..]);
	let mut bwd_key = [0u8; 5];
	bwd_key[0] = b'b';
	bwd_key[1..].copy_from_slice(&(9 + last).to_le_bytes());
	let bwd = storage.service_key(SVC, &bwd_key).expect("backward entry written");
	assert_eq!(&bwd[..4], &last.to_le_bytes()[..]);
	assert_eq!(per_transfer, measured::DEST_HANDLER_PER_TRANSFER);
}

#[test]
fn incoming_transfer_flood_works() {
	// 1024 incoming JAM transfers in one block (phase 2). All are admitted: the
	// first 1000 fill the reserved portion, the rest self-fund. Every append
	// re-reads and re-writes the same growing same-slot bucket.
	let storage = fresh_storage(|s| seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH));
	let items = (0..FLOOD).map(|i| transfer_item(9 + i, 1_000_000)).collect();

	let (outcome, storage, _) = run_block(storage, items, NOW);

	report("incoming_transfer_flood", outcome.gas_used, outcome.elapsed, 0);
	assert_eq!(transfer_chain(&storage).unwrap().count, FLOOD);
	assert_eq!(transfer_bucket(&storage, NOW).unwrap().transfers.len(), FLOOD as usize);
	// ~1.6k/transfer with the D-8 single-bucket write; the pre-D-8 per-transfer
	// rewrite measured 551M — 55x Ga.
	assert_eq!(outcome.gas_used, measured::INCOMING_TRANSFER_FLOOD);
}

#[test]
fn due_assign_flood_works() {
	// Maintenance worst case for the always-accumulate phase (§5.1): every core
	// holds a due pending assign (full 80-hash queue) and a block with zero
	// operands flushes them all — the gas this needs must be covered by the
	// service's `always_acc` allotment, since no operand brings any.
	let storage = fresh_storage(|s| {
		let queue: Vec<Hash> = (0..80u32).map(distinct_hash).collect();
		for core in 0..CORE_COUNT as CoreIndex {
			set_state(
				s,
				&storage_key(Tag::PendingAssigns, &core),
				&PendingAssign { queue: queue.clone(), assigner: None },
			);
		}
		let dirty: PendingAssignCores = (0..CORE_COUNT as CoreIndex)
			.map(|c| (c, NOW))
			.collect::<Vec<_>>()
			.try_into()
			.expect("CORE_COUNT entries fit the bound");
		set_state(s, &storage_key(Tag::PendingAssignCores, &()), &dirty);
	});

	let (outcome, storage, _) = run_block(storage, vec![], NOW);

	report("due_assign_flood", outcome.gas_used, outcome.elapsed, 0);
	let dirty: Option<PendingAssignCores> =
		get_state(&storage, &storage_key(Tag::PendingAssignCores, &()));
	assert!(dirty.is_none_or(|d| d.is_empty()), "all due assigns flushed");
	let gone: Option<PendingAssign> =
		get_state(&storage, &storage_key(Tag::PendingAssigns, &(CORE_COUNT as CoreIndex - 1)));
	assert!(gone.is_none());
	assert_eq!(outcome.gas_used, measured::DUE_ASSIGN_FLOOD);
}

#[test]
fn is_authorized_ed25519_gas_works() {
	use executor::pj;
	use parachain_authorizer_bin::BLOB as AUTHORIZER;
	use parachain_service_bin::mock::{is_authorized_args, make_auth, work_items};
	use parachain_service_interface::types::ParaId;

	let items = work_items(1);
	let (config, token, _) = make_auth(AUTHORIZER, vec![ParaId(0)], &items);
	let (engine, package, storage) = is_authorized_args(AUTHORIZER, config, token, items);

	let outcome = pj::is_authorized(&engine, &package, 0, &storage)
		.expect("is_authorized must succeed with real ed25519 signature");

	let gi = jam_types::max_is_authorized_gas();
	eprintln!(
		"is_authorized_ed25519: gas_used={} ({:.2}x Gi={gi}), elapsed={:?}",
		outcome.gas_used,
		outcome.gas_used as f64 / gi as f64,
		outcome.elapsed,
	);

	assert!(
		outcome.gas_used < gi / 5,
		"is_authorized gas {} must be < Gi/5 = {}",
		outcome.gas_used,
		gi / 5,
	);
	assert_eq!(outcome.gas_used, measured::IS_AUTHORIZED_ED25519);
}
