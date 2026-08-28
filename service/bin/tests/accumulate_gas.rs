//! Worst-case gas benchmarks run through the PVM interpreter.

mod common;

use codec::Encode;
use common::*;
use jam_types::{AccumulateItem, Memo as JamMemo, TransferRecord};
use parachain_service::{
	constants::{CORE_COUNT, MAX_TRANSFER_GAS},
	state::{
		assigns::{PendingAssign, PendingAssignCores},
		storage_key, Tag,
	},
};
use parachain_service_bin::MOCK_DEST_BLOB;
use parachain_service_interface::{
	types::{CoreIndex, Hash, ParaId, ASSET_HUB_PARA_ID},
	upward_message::{UpwardMessage, MAX_UPWARD_MESSAGES_PER_DIGEST},
};

const NOW: u32 = 100;
const PARA: ParaId = ParaId(1000);
const CODE: &[u8] = b"para-1000-code";
const AH_CODE: &[u8] = b"ah-code";
const MAX_UMPS: u32 = MAX_UPWARD_MESSAGES_PER_DIGEST;

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

/// Pinned gas measurements for the benchmarks below.
mod gas {
	/// 1024-solicit digest — the heaviest reachable digest replay.
	pub const MAX_SOLICITS: u64 = 7_789_139;
	/// 1024 KV writes filling the report's elective-data limit.
	pub const MAX_KV_WRITES: u64 = 6_154_121;
	/// 331 outbound transfers to a friendly destination.
	pub const MAX_TRANSFER_OUTS: u64 = 836_682;
	/// 331 outbound transfers to a destination demanding the full cap.
	pub const MAX_GAS_TRANSFER_OUTS: u64 = 831_527;
	/// Gas for 1024 incoming transfers recorded in one bucket write.
	pub const MAX_INCOMING_TRANSFERS: u64 = 1_647_997;
	/// Due `assign` flush for all 341 cores in one block.
	pub const ALL_DUE_ASSIGNS: u64 = 9_943_197;
	/// Marginal cost of a realistic destination's memo handler, per transfer.
	pub const DEST_HANDLER_PER_TRANSFER: u64 = 1_665;
	/// Gas for one Ed25519 authorization.
	pub const IS_AUTHORIZED_ED25519: u64 = 1_010_744;
}

/// Checks the pinned gas measurements against their budgets.
#[test]
fn worst_case_margin_works() {
	let ga = jam_types::max_accumulate_gas();
	let budget = ga - ga / 5;
	assert!(gas::MAX_SOLICITS <= budget);
	assert!(gas::MAX_KV_WRITES <= budget);
	assert!(gas::MAX_TRANSFER_OUTS <= budget);
	assert!(gas::MAX_GAS_TRANSFER_OUTS <= budget);
	assert!(gas::MAX_INCOMING_TRANSFERS <= budget);
	assert!(gas::ALL_DUE_ASSIGNS <= ga);
	assert!(gas::DEST_HANDLER_PER_TRANSFER <= MAX_TRANSFER_GAS / 10);
}

/// Benchmarks a maximum-size digest of validation-code solicitations.
#[test]
fn solicit_bench_works() {
	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let msgs = (0..MAX_UMPS)
		.map(|i| UpwardMessage::Solicit { hash: distinct_hash(i), len: 100.into() })
		.collect();
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", msgs, 0);
	let digest_len = digest.encode().len();
	assert!(digest_len <= MAX_REPORT_ELECTIVE_DATA, "fits into the WR size limit");

	let (outcome, _, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	report("solicit_bench", outcome.gas_used, outcome.elapsed, digest_len);
	assert_eq!(outcome.gas_used, gas::MAX_SOLICITS);
}

/// Benchmarks the gas used to apply the maximum number of small KV writes.
#[test]
fn set_kv_bench_works() {
	// We want to use up the whole WR size limit, so the last UMP must be large.
	const LAST_VALUE_LEN: usize = 33_711;

	let storage = fresh_storage(|s| seed_para(s, PARA, b"genesis", CODE, RICH));
	let msgs = (0..MAX_UMPS)
		.map(|i| UpwardMessage::SetKV {
			key: i.to_le_bytes().to_vec(),
			value: vec![0xAB; if i == MAX_UMPS - 1 { LAST_VALUE_LEN } else { 8 }],
		})
		.collect();
	let digest = ok_digest(PARA, CODE, b"genesis", b"head-1", msgs, 0);
	let digest_len = digest.encode().len();
	assert_eq!(digest_len, MAX_REPORT_ELECTIVE_DATA, "fills up the WR size limit");

	let (outcome, _, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);

	report("set_kv_bench", outcome.gas_used, outcome.elapsed, digest_len);
	assert_eq!(outcome.gas_used, gas::MAX_KV_WRITES);
}

/// Benchmarks the gas used to process the maximum number of outbound transfers.
#[test]
fn transfer_out_bench_works() {
	const TRANSFER_GAS: u64 = 500;

	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, TRANSFER_GAS);
	});
	let msgs = (0..WR_TRANSFER_BENCH)
		.map(|i| transfer_out_msg(42, 1, i as u64, Some(([7; 128], TRANSFER_GAS))))
		.collect();
	let head = [0; 147];
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", &head, msgs, 0);
	let digest_len = digest.encode().len();
	assert_eq!(digest_len, MAX_REPORT_ELECTIVE_DATA);

	let (outcome, _, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let gas_used = outcome.gas_used - u64::from(WR_TRANSFER_BENCH) * TRANSFER_GAS;

	report("transfer_out_bench", gas_used, outcome.elapsed, digest_len);
	assert_eq!(gas_used, gas::MAX_TRANSFER_OUTS);
}

/// Maximum transfers fitting in `Wr`.
const WR_TRANSFER_BENCH: u32 = 331;

/// Benchmarks a report-sized batch of maximum-gas outbound transfers.
#[test]
fn transfer_out_max_gas_bench_works() {
	let storage = fresh_storage(|s| {
		seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH);
		seed_service(s, 42, MAX_TRANSFER_GAS);
	});
	let msgs = (0..WR_TRANSFER_BENCH)
		.map(|i| transfer_out_msg(42, 1, i as u64, Some(([7; 128], MAX_TRANSFER_GAS))))
		.collect();
	let digest = ok_digest(ASSET_HUB_PARA_ID, AH_CODE, b"ah-genesis", b"ah-1", msgs, 0);
	let digest_len = digest.encode().len();
	assert!(digest_len <= MAX_REPORT_ELECTIVE_DATA, "fits into the WR size limit");

	let (outcome, _, _) = accumulate_block(storage, vec![work_item(&digest)], NOW);
	let gas_used = outcome.gas_used - u64::from(WR_TRANSFER_BENCH) * MAX_TRANSFER_GAS;

	report("transfer_out_max_gas_bench", gas_used, outcome.elapsed, digest_len);
	assert_eq!(gas_used, gas::MAX_GAS_TRANSFER_OUTS);
}

/// Builds a mock destination transfer.
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

/// Benchmarks the marginal gas used by the destination transfer handler.
#[test]
fn dest_handler_bench_works() {
	let storage = || fresh_storage_for(MOCK_DEST_BLOB, |_| {});
	let (baseline, _, _) = run_block_for(MOCK_DEST_BLOB, storage(), vec![], NOW);
	let items = (0..MAX_UMPS).map(dest_transfer_item).collect();
	let (outcome, _, _) = run_block_for(MOCK_DEST_BLOB, storage(), items, NOW);

	let per_transfer = (outcome.gas_used - baseline.gas_used) / MAX_UMPS as u64;
	report("dest_handler_bench", outcome.gas_used, outcome.elapsed, 0);
	eprintln!("dest_handler_bench: marginal per-transfer gas: {per_transfer}");
	assert_eq!(per_transfer, gas::DEST_HANDLER_PER_TRANSFER);
}

/// Benchmarks a maximum-size batch of incoming transfers.
#[test]
fn incoming_transfer_bench_works() {
	let storage = fresh_storage(|s| seed_para(s, ASSET_HUB_PARA_ID, b"ah-genesis", AH_CODE, RICH));
	let items = (0..MAX_UMPS).map(|i| transfer_item(9 + i, 1_000_000)).collect();

	let (outcome, _, _) = accumulate_block(storage, items, NOW);

	report("incoming_transfer_bench", outcome.gas_used, outcome.elapsed, 0);
	assert_eq!(outcome.gas_used, gas::MAX_INCOMING_TRANSFERS);
}

/// Benchmarks flushing all due pending assignments in one block.
#[test]
fn due_assign_bench_works() {
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

	let (outcome, _, _) = accumulate_block(storage, vec![], NOW);

	report("due_assign_bench", outcome.gas_used, outcome.elapsed, 0);
	assert_eq!(outcome.gas_used, gas::ALL_DUE_ASSIGNS);
}

/// Benchmarks authorization with a real Ed25519 signature.
#[test]
fn is_authorized_ed25519_gas_works() {
	use executor::pj;
	use parachain_authorizer_bin::BLOB as AUTHORIZER;
	use parachain_service_bin::mock::{is_authorized_args, make_auth, work_items};

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

	assert!(outcome.gas_used < gi / 5);
	assert_eq!(outcome.gas_used, gas::IS_AUTHORIZED_ED25519);
}
