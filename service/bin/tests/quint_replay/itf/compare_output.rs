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

fn heads(state: &Value, codex: &mut Codex) -> Result<BTreeMap<i128, i128>, String> {
	map_entries(field(field(state, "svc")?, "parachains")?)?
		.into_iter()
		.map(|(para, info)| {
			Ok((Codex::para_int(para_id(para, codex)?), integer(field(info, "headData")?)?))
		})
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
	let mut abstract_level = Vec::new();
	let mut concrete_leaves = Vec::new();
	for (model_para, head) in after {
		if before.get(&model_para) == Some(&head) {
			continue;
		}
		let para = u32::from(Codex::para_id(model_para)?);
		let data = Codex::head(head)?;
		let abstract_hash = (model_para * 257 + head)
			.checked_mul(2)
			.and_then(|v| v.checked_add(1))
			.ok_or("abstract leaf overflow")?;
		// SCALE Leaf discriminant=1, para=u32 LE, head=Keccak(raw HeadData).
		let mut leaf = vec![1];
		leaf.extend_from_slice(&para.to_le_bytes());
		leaf.extend_from_slice(&keccak(&data));
		abstract_level.push(abstract_hash);
		concrete_leaves.push((para, keccak(&leaf)));
	}
	concrete_leaves.sort_unstable_by_key(|(para, _)| *para);
	let mut concrete_level: Vec<Hash> = concrete_leaves.into_iter().map(|(_, hash)| hash).collect();
	while abstract_level.len() > 1 {
		let mut next = Vec::new();
		for pair in abstract_level.chunks(2) {
			if pair.len() == 1 {
				next.push(pair[0]);
				continue;
			}
			next.push(pair[0]
				.checked_mul(257)
				.and_then(|v| v.checked_add(pair[1]))
				.and_then(|v| v.checked_mul(2))
				.ok_or("abstract node overflow")?);
		}
		abstract_level = next;
	}
	while concrete_level.len() > 1 {
		let mut next = Vec::new();
		for pair in concrete_level.chunks(2) {
			if pair.len() == 1 {
				next.push(pair[0]);
				continue;
			}
			// SCALE Node discriminant=0 followed by its two fixed-width hashes.
			let mut node = vec![0];
			node.extend_from_slice(&pair[0]);
			node.extend_from_slice(&pair[1]);
			next.push(keccak(&node));
		}
		concrete_level = next;
	}
	Ok(abstract_level.first().copied().zip(concrete_level.first().copied()))
}

pub fn state(
	previous: &Value,
	current: &Value,
	yielded: Option<Hash>,
	mutations: &StateMutations,
	codex: &mut Codex,
	frame: usize,
) -> Result<(), String> {
	// An interrupted invocation restores the root yielded at its last checkpoint,
	// which covers exactly the heads the retained reports changed (§5.1, §5.5).
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
	super::validator_keys::compare(previous, current, mutations, frame)?;
	super::outgoing::compare(current, mutations, frame)?;
	super::services::effects(previous, current, mutations, frame)?;
	// These effects have no representation in the supported replay input domain.
	if !mutations.provided.is_empty() || !mutations.ejected.is_empty() {
		return Err(format!("frame {frame}: unexpected JAM provide/create/eject output"));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn system_heads_use_model_and_concrete_order_independently() {
		let before = json!({"svc":{"parachains":{"#map":[]}}});
		let after = json!({"svc":{"parachains":{"#map":[
			[{"tag":"MkParaId","value":{"#bigint":"1"}},{"headData":{"#bigint":"10"}}],
			[{"tag":"MkParaId","value":{"#bigint":"2"}},{"headData":{"#bigint":"20"}}]
		]}}});
		let (abstract_root, concrete_root) =
			commitment(&before, &after, &mut Codex::default()).unwrap().unwrap();
		assert_eq!(abstract_root, (((257 + 10) * 2 + 1) * 257 + ((514 + 20) * 2 + 1)) * 2);
		let leaf = |para: u32, head: i128| {
			let mut bytes = vec![1];
			bytes.extend_from_slice(&para.to_le_bytes());
			bytes.extend_from_slice(&keccak(&Codex::head(head).unwrap()));
			keccak(&bytes)
		};
		let mut node = vec![0];
		node.extend_from_slice(&leaf(1000, 20));
		node.extend_from_slice(&leaf(1005, 10));
		assert_eq!(concrete_root, keccak(&node));
	}

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
