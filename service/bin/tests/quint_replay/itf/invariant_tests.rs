//! Corrupt only Rust storage: model equality is deliberately not called here.
use super::{codex::Codex, invariant_codes::Eligibility, invariants::*, seed};
use crate::common::{fresh_storage, get_state, set_state};
use jam_node::vm::{StateMutations, Storage};
use parachain_service::{
	constants::*,
	state::{
		assigns::PendingAssign,
		log::LogEntry,
		para_info::ParaInfo,
		preimage_registry::PreimageEntry,
		storage_key,
		transfers::{IncomingTransferBuckets, QueuedTransfer},
		Tag,
	},
};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use serde_json::{json, Value};

fn seeded() -> (Storage, Codex, Value) {
	let trace: Value =
		serde_json::from_str(include_str!("../../fixtures/quint/minimal_replay.itf.json")).unwrap();
	let frame = trace["states"][0].clone();
	let mut codex = Codex::default();
	let storage = fresh_storage(|s| seed::seed(s, &frame, &mut codex).unwrap());
	(storage, codex, frame)
}
fn rejects(name: &str, change: impl FnOnce(&mut Storage, &mut Codex, &mut Value)) {
	let (mut storage, mut codex, mut frame) = seeded();
	state(&storage, &frame, &mut codex, 0).unwrap();
	change(&mut storage, &mut codex, &mut frame);
	let error = state(&storage, &frame, &mut codex, 42).unwrap_err();
	assert!(error.contains(&format!("frame 42: invariant {name} failed")), "{error}");
}
fn info_change(storage: &mut Storage, f: impl FnOnce(&mut ParaInfo)) {
	let key = storage_key(Tag::Parachains, &Codex::para_id(1).unwrap());
	let mut info: ParaInfo = get_state(storage, &key).unwrap();
	f(&mut info);
	set_state(storage, &key, &info);
}
#[test]
fn trivial_works() {
	let (s, mut c, f) = seeded();
	state(&s, &f, &mut c, 0).unwrap();
}
#[test]
fn balance_bounds_errors() {
	rejects("balance_invariant", |s, _, _| {
		info_change(s, |i| i.total_state_balance = i.used_state_balance - 1)
	});
	rejects("used_above_baseline", |s, _, _| info_change(s, |i| i.used_state_balance = 0));
	rejects("used_balance_consistency", |s, _, _| info_change(s, |i| i.used_state_balance += 1));
}
#[test]
fn code_references_errors() {
	rejects("announcement_requires_different_active_code", |s, _, _| {
		info_change(s, |i| i.announced_upgrade = i.validation_code)
	});
	rejects("validation_code_in_registry", |s, _, _| {
		info_change(s, |i| i.validation_code.as_mut().unwrap().len += 1)
	});
	rejects("announced_code_in_registry", |s, _, _| {
		info_change(s, |i| {
			i.announced_upgrade = i.validation_code;
			i.announced_upgrade.as_mut().unwrap().len += 1;
		})
	});
}
#[test]
fn registry_errors() {
	for (name, refs) in
		[("preimage_referencer_consistency", vec![]), ("referencers_are_live", vec![99])]
	{
		rejects(name, |s, c, _| {
			let hash = c.hash(1, 65536).unwrap();
			set_state(
				s,
				&storage_key(Tag::PreimageRegistry, &(hash, 65536u32)),
				&PreimageEntry {
					referencers: refs.into_iter().map(|n| Codex::para_id(n).unwrap()).collect(),
				},
			);
		});
	}
	rejects("preimage_status_domain", |s, c, _| {
		let hash = c.hash(1, 65536).unwrap();
		s.forget(0, MOCK_SERVICE_ID, hash, 65536).unwrap();
		s.commit();
	});
	rejects("unrequested_is_singleton_retention", |s, c, _| {
		let hash = c.hash(1, 65536).unwrap();
		s.provide(1, MOCK_SERVICE_ID, &Codex::blob(1, 65536).unwrap()).unwrap();
		s.commit();
		s.forget(2, MOCK_SERVICE_ID, hash, 65536).unwrap();
		s.commit();
		set_state(
			s,
			&storage_key(Tag::PreimageRegistry, &(hash, 65536u32)),
			&PreimageEntry {
				referencers: [Codex::para_id(1).unwrap(), Codex::para_id(2).unwrap()].into(),
			},
		);
	});
}
#[test]
fn staging_errors() {
	rejects("staged_validator_keys_bounded", |s, _, _| {
		set_state(
			s,
			&storage_key(Tag::StagedValidatorKeys, &()),
			&vec![[0u8; 336]; MAX_STAGED_VALIDATOR_KEYS + 1],
		)
	});
	// Remove all Asset Hub-owned state, leaving an otherwise consistent service.
	rejects("staged_keys_owned_by_asset_hub", |s, c, _| {
		let p = Codex::para_id(2).unwrap();
		let hash = c.hash(2, 65536).unwrap();
		s.remove_service_key(MOCK_SERVICE_ID, &storage_key(Tag::Parachains, &p));
		s.remove_service_key(
			MOCK_SERVICE_ID,
			&storage_key(Tag::PreimageRegistry, &(hash, 65536u32)),
		);
		s.forget(0, MOCK_SERVICE_ID, hash, 65536).unwrap();
		s.commit();
		set_state(s, &storage_key(Tag::StagedValidatorKeys, &()), &vec![[0u8; 336]]);
	});
}
#[test]
fn assignments_errors() {
	rejects("pending_authorizer_cores_consistent", |s, _, _| {
		set_state(s, &storage_key(Tag::PendingAssignCores, &()), &vec![(1u16, 10u32)])
	});
	rejects("pending_authorizer_cores_consistent", |s, _, _| {
		set_state(
			s,
			&storage_key(Tag::PendingAssigns, &1u16),
			&PendingAssign { queue: vec![[0; 32]], assigner: None },
		)
	});
	rejects("pending_authorizer_apply_at_future", |s, _, _| {
		set_state(s, &storage_key(Tag::PendingAssignCores, &()), &vec![(1u16, 0u32)])
	});
	for len in [0, AUTHORIZER_QUEUE_LEN + 1] {
		rejects("pending_authorizer_queue_bounded", |s, _, _| {
			set_state(s, &storage_key(Tag::PendingAssignCores, &()), &vec![(1u16, 10u32)]);
			set_state(
				s,
				&storage_key(Tag::PendingAssigns, &1u16),
				&PendingAssign { queue: vec![[0; 32]; len], assigner: None },
			);
		});
	}
}
#[test]
fn logs_errors() {
	rejects("accumulate_log_batch_nonempty", |s, _, _| {
		set_state(
			s,
			&storage_key(Tag::ParachainLog, &Codex::para_id(1).unwrap()),
			&vec![(0u32, LogEntry::Accumulate { entries: vec![] })],
		)
	});
	let entry = LogEntry::Refine {
		error: parachain_service::work_digest::RefineLog::ValidationCodeLookupFailed,
		auth_trace: Default::default(),
	};
	rejects("parachain_log_only_for_live", |s, c, _| {
		let p = c.register_para(99).unwrap();
		set_state(s, &storage_key(Tag::ParachainLog, &p), &vec![(0u32, entry.clone())]);
	});
	rejects("parachain_log_within_capacity", |s, _, _| {
		set_state(
			s,
			&storage_key(Tag::ParachainLog, &Codex::para_id(1).unwrap()),
			&vec![(0u32, entry.clone()); PARACHAIN_LOG_BYTE_CAP],
		)
	});
	rejects("parachain_log_above_lookup", |s, _, f| {
		f["logPrunedBelow"] =
			json!({"#map": [[{"tag":"MkParaId", "value":{"#bigint":"1"}}, {"#bigint":"2"}]]});
		set_state(
			s,
			&storage_key(Tag::ParachainLog, &Codex::para_id(1).unwrap()),
			&vec![(1u32, entry)],
		);
	});
}
#[test]
fn transfers_errors() {
	let transfer =
		QueuedTransfer { source: 7, amount: 0, to_supervisor_balance: false, memo: [0; 128] };
	for (name, count, len) in [
		("incoming_transfer_count_consistent", 2, 1),
		("incoming_transfer_bucket_bounded", 0, 0),
		("incoming_transfer_bucket_bounded", 513, 513),
	] {
		rejects(name, |s, _, _| {
			set_state(
				s,
				&storage_key(Tag::IncomingTransferBuckets, &()),
				&IncomingTransferBuckets { first_bucket: 0, last_bucket: 0, count },
			);
			set_state(s, &storage_key(Tag::IncomingTransfers, &0u64), &vec![transfer.clone(); len]);
		});
	}
	rejects("incoming_transfers_buckets_consistent", |s, _, _| {
		set_state(
			s,
			&storage_key(Tag::IncomingTransferBuckets, &()),
			&IncomingTransferBuckets { first_bucket: 2, last_bucket: 1, count: 1 },
		)
	});
	rejects("incoming_transfers_buckets_consistent", |s, _, _| {
		set_state(
			s,
			&storage_key(Tag::IncomingTransferBuckets, &()),
			&IncomingTransferBuckets { first_bucket: 0, last_bucket: 0, count: 1 },
		)
	});
	rejects("storage_key_domain", |s, _, _| {
		set_state(s, &storage_key(Tag::IncomingTransfers, &99u64), &vec![transfer])
	});
}
#[test]
fn kv_accounting_errors() {
	rejects("used_balance_consistency", |s, c, _| {
		c.register_kv_key(&[1]).unwrap();
		s.set_service_key(
			MOCK_SERVICE_ID,
			&parachain_service::state::kv::storage_key(Codex::para_id(1).unwrap(), &[1]),
			&[2],
		);
	});
}
#[test]
fn malformed_storage_errors() {
	let (mut s, mut c, f) = seeded();
	let key = storage_key(Tag::Parachains, &Codex::para_id(1).unwrap());
	let mut raw = s.service_key(MOCK_SERVICE_ID, &key).unwrap();
	raw.push(0);
	s.set_service_key(MOCK_SERVICE_ID, &key, &raw);
	assert!(state(&s, &f, &mut c, 0).unwrap_err().contains("trailing storage bytes"));
}
#[test]
fn host_effects_errors() {
	let mut effects_state = StateMutations::new(0);
	effects(&effects_state, 0).unwrap();
	effects_state.keys = Some(Default::default());
	assert!(effects(&effects_state, 0).unwrap_err().contains("designate_only_valid_lengths"));
	effects_state.keys = None;
	effects_state.auths.insert(CORE_COUNT as u16, Default::default());
	assert!(effects(&effects_state, 0).unwrap_err().contains("assign_calls_well_formed"));
}
#[test]
fn catalogue_works() {
	let model = include_str!("../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/invariants.qnt");
	let names: Vec<_> = model
		.lines()
		.filter_map(|line| line.trim().strip_prefix("val "))
		.filter_map(|line| line.split_once(": bool").map(|p| p.0))
		.filter(|name| *name != "invariants")
		.collect();
	assert_eq!(names.len(), 31, "review catalogue whenever the model pin changes");
	let implementation =
		format!("{}{}", include_str!("invariants.rs"), include_str!("invariant_heads.rs"));
	for name in names {
		assert!(implementation.contains(&format!("\"{name}\"")), "missing invariant {name}");
	}
}

#[test]
fn heads_errors() {
	use super::invariant_heads::{commitment, transition};
	let (mut storage, mut codex, mut frame) = seeded();
	let before = heads(&storage, &codex).unwrap();
	let eligibility = Eligibility::snapshot(&storage, &codex).unwrap();
	transition(&before, &eligibility, &storage, &frame, None, &mut codex, 0).unwrap();
	let error = transition(&before, &eligibility, &storage, &frame, Some([0; 32]), &mut codex, 8)
		.unwrap_err();
	assert!(error.contains("head_commitment_matches_changed_heads"), "{error}");
	info_change(&mut storage, |i| i.head_data = Codex::head(1).unwrap());
	let after = heads(&storage, &codex).unwrap();
	let root = commitment(&before, &after);
	let error =
		transition(&before, &eligibility, &storage, &frame, root, &mut codex, 8).unwrap_err();
	assert!(error.contains("head_state_matches_outcomes"), "{error}");
	let trace: Value =
		serde_json::from_str(include_str!("../../fixtures/quint/minimal_replay.itf.json")).unwrap();
	frame["lastStepWorkResults"] = trace["states"][1]["lastStepWorkResults"].clone();
	transition(&before, &eligibility, &storage, &frame, root, &mut codex, 8).unwrap();
	frame["lastStepWorkResults"][0]["result"]["value"]["value"]["parentHeadHash"] =
		json!({"headBytes":{"#bigint":"99"}});
	let error =
		transition(&before, &eligibility, &storage, &frame, root, &mut codex, 8).unwrap_err();
	assert!(error.contains("parent_head_continuity"), "{error}");
}

#[test]
fn inactive_code_heads_works() {
	use super::invariant_heads::{commitment, transition};
	for announced in [false, true] {
		let (mut storage, mut codex, mut frame) = seeded();
		let other = codex.validation_code(777, 65536).unwrap();
		if announced {
			info_change(&mut storage, |info| info.announced_upgrade = Some(other));
		}
		let before = heads(&storage, &codex).unwrap();
		let eligibility = Eligibility::snapshot(&storage, &codex).unwrap();
		let trace: Value =
			serde_json::from_str(include_str!("../../fixtures/quint/minimal_replay.itf.json"))
				.unwrap();
		frame["lastStepWorkResults"] = trace["states"][1]["lastStepWorkResults"].clone();
		let ok = &mut frame["lastStepWorkResults"][0]["result"]["value"]["value"];
		ok["validationCode"] = json!({"vchBytes":{"#bigint":"777"}});
		ok["upwardMessages"] = json!([{"tag":"ParachainSetHead", "value": {
			"paraId":{"tag":"MkParaId", "value":{"#bigint":"2"}}, "newHead":{"#bigint":"99"}
		}}]);
		transition(&before, &eligibility, &storage, &frame, None, &mut codex, 15).unwrap();
		// Neither the candidate nor its messages may change a head with inactive code.
		for (para, head) in [(1, 1), (2, 99)] {
			let mut bad = storage.clone();
			let key = storage_key(Tag::Parachains, &Codex::para_id(para).unwrap());
			let mut info: ParaInfo = get_state(&bad, &key).unwrap();
			info.head_data = Codex::head(head).unwrap();
			set_state(&mut bad, &key, &info);
			let root = commitment(&before, &heads(&bad, &codex).unwrap());
			let error =
				transition(&before, &eligibility, &bad, &frame, root, &mut codex, 15).unwrap_err();
			assert!(error.contains("parent_head_continuity"), "{error}");
		}
	}
}

#[test]
fn same_invocation_code_change_works() {
	use super::{invariant_heads, replay};
	let (storage, mut codex, mut frame) = seeded();
	let before = heads(&storage, &codex).unwrap();
	let mut eligibility = Eligibility::snapshot(&storage, &codex).unwrap();
	let trace: Value =
		serde_json::from_str(include_str!("../../fixtures/quint/minimal_replay.itf.json")).unwrap();
	let mut first = trace["states"][1]["lastStepWorkResults"][0].clone();
	first["result"]["value"]["value"]["upwardMessages"] = json!([{
		"tag":"ParachainSetValidationCode", "value": {
			"paraId":{"tag":"MkParaId", "value":{"#bigint":"1"}},
			"newValidationCode":{"hash":{"vchBytes":{"#bigint":"2"}}, "len":{"#bigint":"65536"}}
		}
	}]);
	let mut second = trace["states"][1]["lastStepWorkResults"][0].clone();
	second["result"]["value"]["value"]["validationCode"] = json!({"vchBytes":{"#bigint":"2"}});
	second["result"]["value"]["value"]["parentHeadHash"] = json!({"headBytes":{"#bigint":"1"}});
	second["result"]["value"]["value"]["headData"] = json!({"#bigint":"2"});
	let mut stale = second.clone();
	stale["result"]["value"]["value"]["validationCode"] = json!({"vchBytes":{"#bigint":"1"}});
	stale["result"]["value"]["value"]["parentHeadHash"] = json!({"headBytes":{"#bigint":"2"}});
	stale["result"]["value"]["value"]["headData"] = json!({"#bigint":"3"});
	frame["lastStepWorkResults"] = json!([first, second, stale]);
	let items = frame["lastStepWorkResults"]
		.as_array()
		.unwrap()
		.iter()
		.map(|result| replay::work_item(result, &mut codex).unwrap())
		.collect::<Vec<_>>();
	let privileges = super::assignments::initial_privileges(storage.clone());
	eligibility.prefixes(&storage, &items, &frame, &privileges, &codex).unwrap();
	let (outcome, after, _) =
		replay::accumulate_block_recovery(storage, items, 0, privileges, None, None).unwrap();
	assert_eq!(
		heads(&after, &codex).unwrap()[&Codex::para_id(1).unwrap()],
		Codex::head(2).unwrap()
	);
	invariant_heads::transition(
		&before,
		&eligibility,
		&after,
		&frame,
		outcome.yielded,
		&mut codex,
		1,
	)
	.unwrap();
}

#[test]
#[ignore = "solicit_implies_registry disabled pending https://github.com/paritytech/parachain-service/issues/54"]
fn solicited_ghost_errors() {
	rejects("solicit_implies_registry", |_, _, f| {
		f["solicitedSet"] = json!({"#set": [{"#tup": [
			{"tag":"MkParaId", "value":{"#bigint":"1"}},
			{"hashBytes":{"#bigint":"77"}}, {"#bigint":"16"}
		]}]});
	});
}

#[test]
fn missing_ghost_errors() {
	// FIXME: Restore solicitedSet coverage when issue #54 is fixed.
	for name in ["logPrunedBelow", "foreignServices"] {
		let (s, mut c, mut f) = seeded();
		f.as_object_mut().unwrap().remove(name);
		assert!(state(&s, &f, &mut c, 0).unwrap_err().contains(name));
	}
}

#[test]
fn foreign_domain_errors() {
	rejects("foreign_excludes_self", |_, _, f| {
		f["foreignServices"] =
			json!({"#map":[[{"tag":"MkServiceId", "value":{"#bigint":"1"}}, {}]]});
	});
	rejects("foreign_supervisor_is_us_or_self", |_, _, f| {
		f["foreignServices"] = json!({"#map":[[
			{"tag":"MkServiceId", "value":{"#bigint":"7"}},
			{"supervisor":{"tag":"MkServiceId", "value":{"#bigint":"8"}}}
		]]});
	});
}

#[test]
fn deregistering_heads_works() {
	use super::invariant_heads::{commitment, transition};
	// Cover both a deregistering origin (its messages must be rejected too)
	// and an active origin forcing a deregistering target's head.
	for frozen in [1, 2] {
		let (mut storage, mut codex, mut frame) = seeded();
		let target = Codex::para_id(frozen).unwrap();
		let key = storage_key(Tag::Parachains, &target);
		let mut info: ParaInfo = get_state(&storage, &key).unwrap();
		info.is_deregistering = true;
		set_state(&mut storage, &key, &info);
		let before = heads(&storage, &codex).unwrap();
		let eligibility = Eligibility::snapshot(&storage, &codex).unwrap();
		let frozen = deregistering(&storage, &codex).unwrap();
		let trace: Value =
			serde_json::from_str(include_str!("../../fixtures/quint/minimal_replay.itf.json"))
				.unwrap();
		frame["lastStepWorkResults"] = trace["states"][1]["lastStepWorkResults"].clone();
		frame["lastStepWorkResults"][0]["result"]["value"]["value"]["upwardMessages"] = json!([
			{"tag": "ParachainSetHead", "value": {
				"paraId": {"tag": "MkParaId", "value": {"#bigint": "2"}},
				"newHead": {"#bigint": "99"}
			}}
		]);
		if !frozen.contains(&Codex::para_id(1).unwrap()) {
			info_change(&mut storage, |i| i.head_data = Codex::head(1).unwrap());
		}
		let root = commitment(&before, &heads(&storage, &codex).unwrap());
		transition(&before, &eligibility, &storage, &frame, root, &mut codex, 8).unwrap();

		// Matching a claimed head must not conceal an illegal write while frozen.
		info.head_data =
			Codex::head(if target == Codex::para_id(1).unwrap() { 1 } else { 99 }).unwrap();
		set_state(&mut storage, &key, &info);
		let root = commitment(&before, &heads(&storage, &codex).unwrap());
		let error =
			transition(&before, &eligibility, &storage, &frame, root, &mut codex, 8).unwrap_err();
		assert!(error.contains("parent_head_continuity"), "{error}");
	}
}
