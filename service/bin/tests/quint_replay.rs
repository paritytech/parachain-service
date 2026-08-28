mod common;

use common::*;
use jam_node::vm::Storage;
use jam_types::AccumulateItem;
use parachain_service::{
	state::{
		log::ParachainLog,
		para_info::{ParaInfo, ValidationCode},
		preimage_registry::PreimageEntry,
		storage_key, Tag,
	},
	state_balance::{baseline_for, preimage_footprint},
};
use parachain_service_bin::mock::provide_preimage;
use parachain_service_interface::types::{Balance, ParaId};
use serde_json::Value;

const CODE: &[u8] = b"para-1000-code";

// --- Quint → Rust codex (simplified for spike) -------------------------------

/// `HeadData` in Quint is an `int`; in Rust it's `BoundedVec<u8, 4KiB>`.
/// Codex: 8-byte LE of the integer.
fn codex_head(n: i128) -> Vec<u8> {
	(n as u64).to_le_bytes().to_vec()
}

/// For the spike, we use `code_ref(CODE)` everywhere and skip the abstract
/// VCH codex. The ITF's `vchBytes` is Quint-only documentation; what matters
/// is that Rust seeds and work items agree on the same real code reference.

fn bigint(v: &Value) -> i128 {
	match v {
		Value::Number(n) => n.as_i64().unwrap() as i128,
		Value::String(s) => s.parse().unwrap(),
		Value::Object(o) => {
			if let Some(s) = o.get("#bigint") {
				s.as_str().unwrap().parse().unwrap()
			} else {
				panic!("expected #bigint, got {v}")
			}
		},
		_ => panic!("expected bigint or number, got {v}"),
	}
}

fn variant_value(v: &Value) -> &Value {
	v.get("value").expect("variant should have 'value'")
}

fn para_id_from_itf(v: &Value) -> ParaId {
	assert_eq!(v["tag"].as_str().unwrap(), "MkParaId");
	ParaId(bigint(&variant_value(v)) as u32)
}

// --- Seed from ITF ------------------------------------------------------------

/// Seed frame 0 from the ITF init state.
///
/// Uses `code_ref(CODE)` for the real validation code — the ITF's abstract
/// `vchBytes` integer is documentation only.
fn seed_from_itf(storage: &mut Storage, init: &Value) {
	let entries = init["svc"]["parachains"]["#map"].as_array().expect("svc.parachains.#map");

	for entry in entries {
		let arr = entry.as_array().unwrap();
		let para = para_id_from_itf(&arr[0]);
		let info_val = &arr[1];

		let head = codex_head(bigint(&info_val["headData"]));
		let total = bigint(&info_val["totalStateBalance"]) as Balance;
		let cref = code_ref(CODE);

		let used = baseline_for(para) + preimage_footprint(cref.len);

		let info = ParaInfo {
			head_data: head.try_into().unwrap(),
			validation_code: Some(ValidationCode { code_ref: cref, pinned: false }),
			pending_upgrade: None,
			total_state_balance: total,
			used_state_balance: used,
			is_deregistering: false,
		};

		set_state(storage, &storage_key(Tag::Parachains, &para), &info);

		let entry = PreimageEntry { referencers: [para].into_iter().collect() };
		set_state(storage, &storage_key(Tag::PreimageRegistry, &(cref.hash.0, cref.len)), &entry);
		provide_preimage(storage, CODE);
	}
}

// --- Build work items from ITF ------------------------------------------------

fn work_items_from_itf(work_results: &[Value]) -> Vec<AccumulateItem> {
	work_results
		.iter()
		.map(|wr| {
			let result = &wr["result"];
			assert_eq!(result["tag"].as_str().unwrap(), "MkOk");
			let digest = variant_value(result);

			let para = para_id_from_itf(&digest["paraId"]);

			// Parent head hash from ITF via codex.
			let parent_head_bytes = codex_head(bigint(&digest["parentHeadHash"]["hashBytes"]));

			// New head data from ITF via codex.
			let new_head = codex_head(bigint(&digest["headData"]));

			let lookup_anchor = bigint(&digest["lookupAnchor"]) as u32;

			let digest =
				ok_digest(para, CODE, &parent_head_bytes, &new_head, vec![], lookup_anchor);
			work_item(&digest)
		})
		.collect()
}

// --- Compare ------------------------------------------------------------------

/// Compare ITF expected state against Rust storage.
fn assert_state_eq(storage: &Storage, expected: &Value, slot_label: &str) -> Result<(), String> {
	let entries = expected["svc"]["parachains"]
		.get("#map")
		.and_then(|v| v.as_array())
		.ok_or("expected svc.parachains.#map")?;

	for entry in entries {
		let arr = entry.as_array().unwrap();
		let para = para_id_from_itf(&arr[0]);
		let info_val = &arr[1];
		let n = para.0;

		let expected_head = codex_head(bigint(&info_val["headData"]));
		let expected_total = bigint(&info_val["totalStateBalance"]) as Balance;

		let rust_info = para_info(storage, para)
			.ok_or(format!("[{slot_label}] para {n} missing from storage"))?;

		if rust_info.head_data[..] != expected_head[..] {
			return Err(format!(
				"[{slot_label}] para {n} head_data: expected {expected_head:?}, got {:?}",
				&rust_info.head_data[..]
			));
		}
		if rust_info.total_state_balance != expected_total {
			return Err(format!(
				"[{slot_label}] para {n} total_state_balance: expected {expected_total}, got {}",
				rust_info.total_state_balance
			));
		}
		if rust_info.used_state_balance !=
			baseline_for(para) + preimage_footprint(code_ref(CODE).len)
		{
			return Err(format!(
				"[{slot_label}] para {n} used_state_balance: expected {}, got {}",
				baseline_for(para) + preimage_footprint(code_ref(CODE).len),
				rust_info.used_state_balance
			));
		}
		if rust_info.is_deregistering {
			return Err(format!("[{slot_label}] para {n} is_deregistering is true"));
		}

		// Log should be empty.
		let log: ParachainLog =
			get_state(storage, &storage_key(Tag::ParachainLog, &para)).unwrap_or_default();
		if !log.is_empty() {
			return Err(format!("[{slot_label}] para {n} parachain_log not empty: {log:?}"));
		}
	}

	Ok(())
}

// --- The test ----------------------------------------------------------------

#[test]
fn minimal_replay_works() {
	let fixture = include_str!("fixtures/quint/minimal_replay.itf.json");
	let itf: Value = serde_json::from_str(fixture).expect("valid ITF JSON");
	let states = itf["states"].as_array().expect("states array");
	assert!(states.len() >= 2, "need at least init + 1 block");
	let init_head = bigint(&states[0]["svc"]["parachains"]["#map"][0][1]["headData"]);
	let expected_head = bigint(&states[1]["svc"]["parachains"]["#map"][0][1]["headData"]);
	assert_ne!(init_head, expected_head, "fixture must describe a state transition");

	// 1. Seed frame 0 from ITF.
	let storage = fresh_storage(|s| seed_from_itf(s, &states[0]));

	// 2. Build work items from state[1]'s lastStepWorkResults.
	let block_state = &states[1];
	let work_results = block_state["lastStepWorkResults"].as_array().expect("lastStepWorkResults");
	assert!(!work_results.is_empty(), "block frame must contain work results");
	let items = work_items_from_itf(work_results);
	assert_eq!(items.len(), work_results.len(), "every work result must be replayed");

	// 3. Run accumulate.
	let slot = bigint(&block_state["now"]) as u32;
	let (outcome, storage, _) = accumulate_block(storage, items, slot);
	assert!(outcome.gas_used > 0, "accumulate should use gas");
	let replayed_head = para_info(&storage, ParaId(1)).expect("replayed para exists").head_data;
	assert_ne!(&replayed_head[..], &codex_head(init_head), "replay must change the head");

	// 4. Compare against expected state.
	assert_state_eq(&storage, block_state, "state[1]").expect("state mismatch");
}
