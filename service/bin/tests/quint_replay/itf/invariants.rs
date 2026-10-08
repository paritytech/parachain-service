//! Independent predicates over decoded Rust storage, named after invariants.qnt.
//! The codex supplies key preimages (JAM hashes keys), never expected values.
//! Ghost solicit/pruning history comes from the trace. Host limitations are
//! explicit: supervisor balances/links are not represented by this JAM revision.
use parachain_service_core::types::validation_code_hash_bytes;
use std::collections::{BTreeMap, BTreeSet};

use codec::{Decode, Encode};
use jam_node::vm::{StateMutations, Storage};
use parachain_service::{
	constants::*,
	state::{
		assigns::PendingAssign,
		kv,
		log::{LogEntry, ParachainLog},
		para_info::ParaInfo,
		preimage_registry::PreimageEntry,
		storage_key,
		transfers::{IncomingTransferBuckets, QueuedTransfer},
		Tag,
	},
	state_balance::{baseline_for, excess_transfer_footprint},
};
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_core::types::{HeadData, ParaId, ASSET_HUB_PARA_ID};
use serde_json::Value;

use super::{codex::Codex, replay::*};

pub(super) fn check(
	ok: bool,
	name: &str,
	frame: usize,
	detail: impl std::fmt::Debug,
) -> Result<(), String> {
	if ok {
		Ok(())
	} else {
		Err(format!("frame {frame}: invariant {name} failed: {detail:?}"))
	}
}

pub(super) fn read<T: Decode>(storage: &Storage, key: &[u8]) -> Result<Option<T>, String> {
	storage
		.service_key(MOCK_SERVICE_ID, key)
		.map(|raw| {
			let mut bytes = &raw[..];
			let value = T::decode(&mut bytes)
				.map_err(|e| format!("invariant storage decode {key:?}: {e}"))?;
			if !bytes.is_empty() {
				return Err(format!("invariant trailing storage bytes {key:?}"));
			}
			Ok(value)
		})
		.transpose()
}

pub fn heads(storage: &Storage, codex: &Codex) -> Result<BTreeMap<ParaId, HeadData>, String> {
	codex
		.paras()
		.filter_map(|p| match read::<ParaInfo>(storage, &storage_key(Tag::Parachains, &p)) {
			Ok(Some(info)) => Some(Ok((p, info.head_data))),
			Ok(None) => None,
			Err(e) => Some(Err(e)),
		})
		.collect()
}

/// Snapshot lifecycle flags before executing the invocation. Post-state flags
/// cannot decide whether a candidate preceding a cleanup was accepted.
pub fn deregistering(storage: &Storage, codex: &Codex) -> Result<BTreeSet<ParaId>, String> {
	let mut result = BTreeSet::new();
	for p in codex.paras() {
		if read::<ParaInfo>(storage, &storage_key(Tag::Parachains, &p))?
			.is_some_and(|info| info.is_deregistering)
		{
			result.insert(p);
		}
	}
	Ok(result)
}

pub fn state(
	storage: &Storage,
	current: &Value,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	let now = bounded_integer::<u32>(field(current, "now")?, "now")?;
	state_at(storage, current, codex, frame, now)
}

/// Due assignments must be in the future relative to the last completed
/// always-accumulate phase. Rollback (and later external provision) cannot
/// advance this watermark; all other state predicates still run normally.
pub fn state_at(
	storage: &Storage,
	current: &Value,
	codex: &mut Codex,
	frame: usize,
	now: u32,
) -> Result<(), String> {
	let mut paras = BTreeMap::new();
	for p in codex.paras() {
		if let Some(info) = read::<ParaInfo>(storage, &storage_key(Tag::Parachains, &p))? {
			paras.insert(p, info);
		}
	}
	let mut registry = BTreeMap::new();
	for (abstract_hash, hash, len) in codex.preimages() {
		let entry =
			read::<PreimageEntry>(storage, &storage_key(Tag::PreimageRegistry, &(hash, len)))?;
		// The boot code request belongs to host setup, unless explicitly registered.
		if abstract_hash == 0 && entry.is_none() {
			continue;
		}
		let request = storage.lookup_request(MOCK_SERVICE_ID, hash, len);
		check(entry.is_some() == request.is_some(), "preimage_status_domain", frame, (hash, len))?;
		if let Some(entry) = entry {
			check(
				!entry.referencers.is_empty(),
				"preimage_referencer_consistency",
				frame,
				(hash, len),
			)?;
			check(
				entry.referencers.iter().all(|p| paras.contains_key(p)),
				"referencers_are_live",
				frame,
				&entry,
			)?;
			check(
				request.as_ref().is_none_or(|r| r.0.len() != 2) || entry.referencers.len() == 1,
				"unrequested_is_singleton_retention",
				frame,
				&entry,
			)?;
			registry.insert((hash, len), entry);
		}
	}
	let endpoints: Option<IncomingTransferBuckets> =
		read(storage, &storage_key(Tag::IncomingTransferBuckets, &()))?;
	let mut count = 0u64;
	if let Some(e) = endpoints {
		check(
			e.first_bucket <= e.last_bucket &&
				e.last_bucket - e.first_bucket < storage.keys().count() as u64,
			"incoming_transfers_buckets_consistent",
			frame,
			e,
		)?;
		for id in e.first_bucket..=e.last_bucket {
			let bucket: Option<Vec<QueuedTransfer>> =
				read(storage, &storage_key(Tag::IncomingTransfers, &id))?;
			check(bucket.is_some(), "incoming_transfers_buckets_consistent", frame, id)?;
			let bucket = bucket.unwrap();
			check(
				!bucket.is_empty() && bucket.len() <= MAX_TRANSFERS_PER_BUCKET as usize,
				"incoming_transfer_bucket_bounded",
				frame,
				id,
			)?;
			count += bucket.len() as u64;
		}
		check(
			count == u64::from(e.count),
			"incoming_transfer_count_consistent",
			frame,
			(count, e.count),
		)?;
	}
	for (p, info) in &paras {
		check(info.used_state_balance <= info.total_state_balance, "balance_invariant", frame, p)?;
		check(
			info.announced_upgrade
				.is_none_or(|a| info.validation_code.is_some_and(|v| a != v)),
			"announcement_requires_different_active_code",
			frame,
			p,
		)?;
		for (name, code) in [
			("validation_code_in_registry", info.validation_code),
			("announced_code_in_registry", info.announced_upgrade),
		] {
			if let Some(code) = code {
				check(
					registry
						.get(&(validation_code_hash_bytes(&code.hash), code.len))
						.is_some_and(|e| e.referencers.contains(p)),
					name,
					frame,
					p,
				)?;
			}
		}
		let baseline = baseline_for(*p);
		check(info.used_state_balance >= baseline, "used_above_baseline", frame, p)?;
		// Independent sums use spec costs rather than the service's charging helpers.
		let refs: u64 = registry
			.iter()
			.filter(|(_, e)| e.referencers.contains(p))
			.map(|((_, len), _)| 187 + u64::from(*len))
			.sum();
		let mut kv_cost = 0u64;
		for key in codex.kv_keys() {
			if let Some(value) = storage.service_key(MOCK_SERVICE_ID, &kv::storage_key(*p, key)) {
				kv_cost += 49 + key.len() as u64 + value.len() as u64;
			}
		}
		let excess = if *p == ASSET_HUB_PARA_ID { excess_transfer_footprint(count) } else { 0 };
		check(
			info.used_state_balance == baseline + refs + kv_cost + excess,
			"used_balance_consistency",
			frame,
			(p, info.used_state_balance, baseline + refs + kv_cost + excess),
		)?;
	}
	let staged: Vec<[u8; 336]> =
		read(storage, &storage_key(Tag::StagedValidatorKeys, &()))?.unwrap_or_default();
	check(
		staged.len() <= MAX_STAGED_VALIDATOR_KEYS,
		"staged_validator_keys_bounded",
		frame,
		staged.len(),
	)?;
	check(
		staged.is_empty() ||
			paras
				.get(&ASSET_HUB_PARA_ID)
				.is_some_and(|p| p.used_state_balance >= 336 * staged.len() as u64),
		"staged_keys_owned_by_asset_hub",
		frame,
		staged.len(),
	)?;
	let due: Vec<(u16, u32)> =
		read(storage, &storage_key(Tag::PendingAssignCores, &()))?.unwrap_or_default();
	let cores: BTreeSet<_> = due.iter().map(|(c, _)| *c).collect();
	check(
		cores.len() == due.len() && cores.iter().all(|c| usize::from(*c) < CORE_COUNT),
		"pending_authorizer_cores_consistent",
		frame,
		&due,
	)?;
	check(
		due.iter().all(|(_, slot)| *slot > now),
		"pending_authorizer_apply_at_future",
		frame,
		&due,
	)?;
	for core in 0..CORE_COUNT as u16 {
		let pending: Option<PendingAssign> =
			read(storage, &storage_key(Tag::PendingAssigns, &core))?;
		check(
			pending.is_some() == cores.contains(&core),
			"pending_authorizer_cores_consistent",
			frame,
			core,
		)?;
		if let Some(pending) = pending {
			check(
				!pending.queue.is_empty() && pending.queue.len() <= AUTHORIZER_QUEUE_LEN,
				"pending_authorizer_queue_bounded",
				frame,
				core,
			)?;
		}
	}
	let mut thresholds = BTreeMap::new();
	{
		let pruned = field(current, "logPrunedBelow")?;
		for (p, slot) in map_entries(pruned)? {
			thresholds.insert(para_id(p, codex)?, bounded_integer::<u32>(slot, "pruning slot")?);
		}
	}
	for p in codex.paras() {
		if let Some(log) = read::<ParachainLog>(storage, &storage_key(Tag::ParachainLog, &p))? {
			// Empty lists are an allowed storage representation of an absent log.
			check(
				log.is_empty() || paras.contains_key(&p),
				"parachain_log_only_for_live",
				frame,
				p,
			)?;
			check(
				log.encoded_size() <= PARACHAIN_LOG_BYTE_CAP,
				"parachain_log_within_capacity",
				frame,
				p,
			)?;
			check(
				log.iter().all(
					|(_, e)| !matches!(e, LogEntry::Accumulate { entries } if entries.is_empty()),
				),
				"accumulate_log_batch_nonempty",
				frame,
				p,
			)?;
			check(
				log.iter().all(|(slot, _)| *slot >= *thresholds.get(&p).unwrap_or(&0)),
				"parachain_log_above_lookup",
				frame,
				p,
			)?;
		}
	}
	// The pinned oracle records successful preimage operations in message order.
	{
		let solicited = field(current, "solicitedSet")?;
		for triple in set_values(solicited)? {
			let triple = tuple(triple)?;
			if triple.len() != 3 {
				return Err("solicitedSet must contain triples".into());
			}
			let p = para_id(&triple[0], codex)?;
			let len = bounded_integer::<u32>(&triple[2], "solicit length")?;
			let hash = codex.hash(integer(field(&triple[1], "hashBytes")?)?, len)?;
			check(
				!paras.contains_key(&p) ||
					registry.get(&(hash, len)).is_some_and(|e| e.referencers.contains(&p)),
				"solicit_implies_registry",
				frame,
				p,
			)?;
		}
	}
	storage_domain(storage, codex, endpoints, frame)?;
	host_accounts(storage, current, frame)?;
	Ok(())
}

/// Unsigned JAM balances make non-negativity structural. The host has no
/// supervisor balances or mutable supervisor links; only self-supervised
/// foreign accounts are used; parentage and creation effects are checked separately.
fn host_accounts(storage: &Storage, current: &Value, frame: usize) -> Result<(), String> {
	let own = storage
		.service(MOCK_SERVICE_ID)
		.ok_or("jam_balances_nonnegative: missing service")?;
	let _: u64 = own.balance;
	{
		let foreign = field(current, "foreignServices")?;
		for (id, value) in map_entries(foreign)? {
			let id = super::assignments::service_id(id)?;
			check(id != MOCK_SERVICE_ID, "foreign_excludes_self", frame, id)?;
			check(
				super::assignments::service_id(field(value, "supervisor")?)? == id,
				"foreign_supervisor_is_us_or_self",
				frame,
				"host only represents self-supervised foreign fixtures",
			)?;
			let account = storage.service(id).ok_or_else(|| format!("frame {frame}: invariant jam_balances_nonnegative: foreign account {id} not represented by host"))?;
			let _: u64 = account.balance;
			check(
				bounded_integer::<u64>(
					field(field(value, "account")?, "supervisorBalance")?,
					"supervisor balance",
				)? == 0,
				"jam_balances_nonnegative",
				frame,
				"supervisor balances unsupported",
			)?;
		}
	}
	Ok(())
}

/// Audit the complete hashed storage domain without using expected model maps.
/// Unknown keys cannot be inverted; rejecting them prevents invisible orphan
/// entries from escaping the predicates that walk known logical keys.
fn storage_domain(
	storage: &Storage,
	codex: &Codex,
	endpoints: Option<IncomingTransferBuckets>,
	frame: usize,
) -> Result<(), String> {
	use jam_std_common::{ServiceKey, StorageKey};
	let mut allowed = BTreeSet::<StorageKey>::new();
	let mut allow = |key: Vec<u8>| {
		allowed.insert(ServiceKey::Value { id: MOCK_SERVICE_ID, key: &key }.into());
	};
	for p in codex.paras() {
		allow(storage_key(Tag::Parachains, &p));
		allow(storage_key(Tag::ParachainLog, &p));
		for key in codex.kv_keys() {
			allow(kv::storage_key(p, key));
		}
	}
	for (_, hash, len) in codex.preimages() {
		allow(storage_key(Tag::PreimageRegistry, &(hash, len)));
	}
	for core in 0..CORE_COUNT as u16 {
		allow(storage_key(Tag::PendingAssigns, &core));
	}
	for tag in [Tag::PendingAssignCores, Tag::StagedValidatorKeys, Tag::IncomingTransferBuckets] {
		allow(storage_key(tag, &()));
	}
	if let Some(e) = endpoints {
		for id in e.first_bucket..=e.last_bucket {
			allow(storage_key(Tag::IncomingTransfers, &id));
		}
	}
	let mut preimages = codex.preimages();
	let boot = parachain_service_bin::blob();
	preimages.push((0, jam_std_common::hash_raw(&boot), boot.len() as u32));
	for (_, hash, len) in preimages {
		allowed.insert(ServiceKey::Request { id: MOCK_SERVICE_ID, hash, len }.into());
		allowed.insert(ServiceKey::Preimage { id: MOCK_SERVICE_ID, hash }.into());
	}
	let id = MOCK_SERVICE_ID.to_le_bytes();
	for key in storage.keys() {
		if [key[0], key[2], key[4], key[6]] == id {
			check(allowed.contains(&StorageKey::from(key)), "storage_key_domain", frame, key)?;
		}
	}
	Ok(())
}

pub fn effects(mutations: &StateMutations, frame: usize) -> Result<(), String> {
	check(
		mutations.keys.as_ref().is_none_or(|keys| {
			(6..=MAX_STAGED_VALIDATOR_KEYS).contains(&keys.len()) && keys.len() % 3 == 0
		}),
		"designate_only_valid_lengths",
		frame,
		mutations.keys.as_ref().map(|k| k.len()),
	)?;
	check(
		mutations.auths.keys().all(|c| usize::from(*c) < CORE_COUNT),
		"assign_calls_well_formed",
		frame,
		&mutations.auths,
	)?;
	// The complete creation set and account metadata are checked by services.rs.
	check(
		!mutations.created.contains(&MOCK_SERVICE_ID),
		"foreign_excludes_self",
		frame,
		&mutations.created,
	)?;
	Ok(())
}
