//! Output codex: abstract Quint Merkle hashes and final JAM host effects.
use std::collections::BTreeMap;

use jam_node::vm::StateMutations;
use jam_types::Hash;
use serde_json::Value;
use tiny_keccak::{Hasher, Keccak};

use super::{codex::Codex, replay::*};

fn keccak(bytes: &[u8]) -> Hash {
	let mut hash = [0; 32];
	let mut k = Keccak::v256();
	k.update(bytes);
	k.finalize(&mut hash);
	hash
}

fn heads(state: &Value, codex: &mut Codex) -> Result<BTreeMap<u32, i128>, String> {
	map_entries(field(field(state, "svc")?, "parachains")?)?
		.into_iter()
		.map(|(para, info)| Ok((para_id(para, codex)?.0, integer(field(info, "headData")?)?)))
		.collect()
}

/// Build both representations from the model state, without using the service's
/// commitment implementation. The abstract hash is not a concrete hash integer.
fn commitment(
	previous: &Value,
	current: &Value,
	codex: &mut Codex,
) -> Result<Option<(i128, Hash)>, String> {
	let before = heads(previous, codex)?;
	let after = heads(current, codex)?;
	let mut level = Vec::new();
	for (para, head) in after {
		if before.get(&para) == Some(&head) {
			continue;
		}
		let data = Codex::head(head)?;
		let abstract_hash =
			(i128::from(para) * 257 + head).checked_mul(2).ok_or("abstract leaf overflow")?;
		// SCALE Leaf discriminant=1, para=u32 LE, head=Keccak(raw HeadData).
		let mut leaf = vec![1];
		leaf.extend_from_slice(&para.to_le_bytes());
		leaf.extend_from_slice(&keccak(&data));
		level.push((abstract_hash, keccak(&leaf)));
	}
	while level.len() > 1 {
		let mut next = Vec::new();
		for pair in level.chunks(2) {
			if pair.len() == 1 {
				next.push(pair[0]);
				continue;
			}
			let abstract_hash = pair[0]
				.0
				.checked_mul(257)
				.and_then(|v| v.checked_add(pair[1].0))
				.and_then(|v| v.checked_mul(2))
				.and_then(|v| v.checked_add(1))
				.ok_or("abstract node overflow")?;
			// SCALE Node discriminant=0 followed by its two fixed-width hashes.
			let mut node = vec![0];
			node.extend_from_slice(&pair[0].1);
			node.extend_from_slice(&pair[1].1);
			next.push((abstract_hash, keccak(&node)));
		}
		level = next;
	}
	Ok(level.first().copied())
}

pub fn state(
	previous: &Value,
	current: &Value,
	yielded: Option<Hash>,
	mutations: &StateMutations,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	let root = commitment(previous, current, codex)?;
	let expected = match variant(field(current, "lastHeadRoot")?)? {
		("None", _) => None,
		("Some", hash) => Some(integer(
			hash.get("merkleBytes")
				.or_else(|| hash.get("hashBytes"))
				.ok_or("missing Merkle hash")?,
		)?),
		(tag, _) => return Err(format!("frame {frame}: invalid lastHeadRoot variant {tag}")),
	};
	if expected != root.map(|v| v.0) {
		return Err(format!(
			"frame {frame}: lastHeadRoot differs from Quint changed-head commitment"
		));
	}
	if yielded != root.map(|v| v.1) {
		return Err(format!(
			"frame {frame}: returned head commitment differs; Quint mapped={:?}; Rust={yielded:?}",
			root.map(|v| v.1)
		));
	}
	staging(previous, current, mutations, frame)?;
	// These effects have no representation in the supported replay input domain.
	if !mutations.transfers.is_empty() ||
		!mutations.provided.is_empty() ||
		!mutations.created.is_empty() ||
		!mutations.ejected.is_empty()
	{
		return Err(format!("frame {frame}: unexpected JAM transfer/provide/create/eject output"));
	}
	Ok(())
}

fn staging(
	previous: &Value,
	current: &Value,
	mutations: &StateMutations,
	frame: usize,
) -> Result<(), String> {
	// No validator-key messages are supported yet. Fail closed on both a model
	// staging change and a Rust designate, including designation of the same set.
	let before = field(previous, "jamStagingSet")?
		.as_array()
		.ok_or("jamStagingSet must be a list")?;
	let after = field(current, "jamStagingSet")?
		.as_array()
		.ok_or("jamStagingSet must be a list")?;
	if before != after || mutations.keys.is_some() {
		return Err(format!(
			"frame {frame}: jamStagingSet differs or requires a designation input codex"
		));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn fixture() -> Value {
		serde_json::from_str(include_str!("../../fixtures/quint/minimal_replay.itf.json")).unwrap()
	}

	#[test]
	fn mutated_model_root_errors() {
		let mut trace = fixture();
		document_trace(&trace).unwrap();
		trace["states"][1]["lastHeadRoot"] =
			json!({"tag":"Some", "value":{"merkleBytes":{"#bigint":"999"}}});
		assert!(document_trace(&trace).unwrap_err().contains("frame 1: lastHeadRoot differs"));
		trace["states"][1]["lastHeadRoot"] = json!({"tag":"None", "value":{"#tup":[]}});
		assert!(document_trace(&trace).unwrap_err().contains("frame 1: lastHeadRoot differs"));
	}

	#[test]
	fn returned_hash_errors() {
		let trace = fixture();
		let (before, after) = (&trace["states"][0], &trace["states"][1]);
		let mut codex = Codex::default();
		let root = commitment(before, after, &mut codex).unwrap().unwrap().1;
		let mutations = StateMutations::new(0);
		state(before, after, Some(root), &mutations, &mut codex, 1).unwrap();
		let mut wrong = root;
		wrong[0] ^= 1;
		for yielded in [None, Some(wrong)] {
			assert!(state(before, after, yielded, &mutations, &mut codex, 1)
				.unwrap_err()
				.contains("returned head commitment differs"));
		}
	}

	#[test]
	fn unchanged_head_output_errors() {
		let trace = fixture();
		let before = &trace["states"][0];
		let mut codex = Codex::default();
		let mutations = StateMutations::new(0);
		state(before, before, None, &mutations, &mut codex, 1).unwrap();
		assert!(state(before, before, Some([0; 32]), &mutations, &mut codex, 1)
			.unwrap_err()
			.contains("returned head commitment differs"));
	}

	#[test]
	fn model_assignment_and_staging_errors() {
		for field in ["lastStepAssigns", "jamStagingSet"] {
			let mut trace = fixture();
			trace["states"][1][field] = json!([{"#bigint":"7"}]);
			assert!(document_trace(&trace).unwrap_err().contains(field));
		}
	}

	#[test]
	fn unexpected_host_effects_errors() {
		let trace = fixture();
		let before = &trace["states"][0];
		for kind in 0..3 {
			let mut mutations = StateMutations::new(0);
			match kind {
				0 => mutations.keys = Some(Default::default()),
				1 => {
					mutations.created.insert(99);
				},
				_ => {
					mutations.provided.insert((0, vec![1]));
				},
			}
			assert!(
				state(before, before, None, &mutations, &mut Codex::default(), 1).is_err(),
				"effect {kind}"
			);
		}
	}
}
