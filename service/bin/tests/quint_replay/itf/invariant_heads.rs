//! Transition predicates use before/after Rust heads, never model svc heads.
use super::{
	codex::Codex,
	invariant_codes::Eligibility,
	invariants::{check, heads},
	replay::*,
};
use jam_node::vm::Storage;
use jam_types::Hash;
use parachain_service_core::types::{HeadData, ParaId};
use serde_json::Value;
use std::collections::BTreeMap;
use tiny_keccak::{Hasher, Keccak};

fn keccak(bytes: &[u8]) -> Hash {
	let mut hash = [0; 32];
	let mut k = Keccak::v256();
	k.update(bytes);
	k.finalize(&mut hash);
	hash
}

pub fn commitment(
	before: &BTreeMap<ParaId, HeadData>,
	after: &BTreeMap<ParaId, HeadData>,
) -> Option<Hash> {
	let mut level: Vec<_> = after
		.iter()
		.filter(|(p, h)| before.get(p) != Some(h))
		.map(|(p, h)| {
			let mut leaf = vec![1];
			leaf.extend_from_slice(&p.0.to_le_bytes());
			leaf.extend_from_slice(&keccak(h));
			keccak(&leaf)
		})
		.collect();
	while level.len() > 1 {
		level = level
			.chunks(2)
			.map(|pair| {
				if pair.len() == 1 {
					return pair[0];
				}
				let mut node = vec![0];
				node.extend_from_slice(&pair[0]);
				node.extend_from_slice(&pair[1]);
				keccak(&node)
			})
			.collect();
	}
	level.first().copied()
}

pub fn transition(
	before: &BTreeMap<ParaId, HeadData>,
	eligibility: &Eligibility,
	storage: &Storage,
	current: &Value,
	yielded: Option<Hash>,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	let after = heads(storage, codex)?;
	check(
		yielded ==
			if super::gas::interrupted(current)?.is_some() {
				None
			} else {
				commitment(before, &after)
			},
		"head_commitment_matches_changed_heads",
		frame,
		yielded,
	)?;
	let mut replayed = before.clone();
	let mut claims = Vec::new();
	for (index, result) in field(current, "lastStepWorkResults")?
		.as_array()
		.ok_or("work results must be a list")?
		.iter()
		.enumerate()
	{
		let (tag, digest) = variant(field(result, "result")?)?;
		if tag != "WorkOk" {
			continue;
		}
		let (tag, ok) = variant(digest)?;
		if tag != "Ok" {
			continue;
		}
		let p = para_id(field(ok, "paraId")?, codex)?;
		let head = Codex::head(integer(field(ok, "headData")?)?)?;
		claims.push((p, head.clone()));
		let parent = field(ok, "parentHeadHash")?;
		let parent = Codex::head(integer(
			parent
				.get("headBytes")
				.or_else(|| parent.get("hashBytes"))
				.ok_or("parent head missing")?,
		)?)?;
		// A retained ParaInfo is not necessarily eligible for more work (§6.4).
		let code = codex.code_hash(integer(field(field(ok, "validationCode")?, "vchBytes")?)?)?;
		let accepted = !eligibility.deregistering.contains(&p) &&
			eligibility.code(index, p) == Some(code) &&
			replayed.get(&p) == Some(&parent);
		if accepted && !super::storage_budget::failed_head(current, index)? {
			replayed.insert(p, head);
		}
		for (message_index, message) in field(ok, "upwardMessages")?
			.as_array()
			.ok_or("messages must be a list")?
			.iter()
			.enumerate()
		{
			let (tag, payload) = variant(message)?;
			if tag == "ParachainSetHead" {
				let target = para_id(field(payload, "paraId")?, codex)?;
				let head = Codex::head(integer(field(payload, "newHead")?)?)?;
				claims.push((target, head.clone()));
				if accepted &&
					!eligibility.deregistering.contains(&target) &&
					replayed.contains_key(&target) &&
					!super::storage_budget::failed_message(current, index, message_index)?
				{
					replayed.insert(target, head);
				}
			}
		}
	}
	for (p, old) in before {
		if let Some(new) = after.get(p) {
			check(
				old == new || claims.contains(&(*p, new.clone())),
				"head_state_matches_outcomes",
				frame,
				p,
			)?;
			check(replayed.get(p) == Some(new), "parent_head_continuity", frame, p)?;
		}
	}
	Ok(())
}

/// Provisioning and no-op frames execute no work and return no commitment.
/// Do not reuse lastStepWorkResults: historical provision traces retain it.
pub fn unchanged(
	before: &BTreeMap<ParaId, HeadData>,
	storage: &Storage,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	transition(
		before,
		&Eligibility::default(),
		storage,
		&serde_json::json!({"lastStepWorkResults": []}),
		None,
		codex,
		frame,
	)
}
