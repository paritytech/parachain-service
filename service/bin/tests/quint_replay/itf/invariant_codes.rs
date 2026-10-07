//! Active-code inputs to the head predicate, read from Rust storage.
use std::collections::{BTreeMap, BTreeSet};

use jam_codec::{Decode, Encode};
use jam_node::vm::Storage;
use jam_types::AccumulateItem;
use parachain_service::state::{para_info::ParaInfo, storage_key, Tag};
use parachain_service_core::types::{ParaId, ValidationCodeHash};
use serde_json::Value;

use super::{codex::Codex, gas, invariants, replay::*};

type Codes = BTreeMap<ParaId, ValidationCodeHash>;

fn codes(storage: &Storage, codex: &Codex) -> Result<Codes, String> {
	let mut codes = Codes::new();
	for para in codex.paras() {
		if let Some(info) =
			invariants::read::<ParaInfo>(storage, &storage_key(Tag::Parachains, &para))?
		{
			if let Some(code) = info.validation_code {
				codes.insert(para, code.hash);
			}
		}
	}
	Ok(codes)
}

#[derive(Default)]
pub struct Eligibility {
	pub deregistering: BTreeSet<ParaId>,
	initial: Codes,
	before_report: BTreeMap<usize, Codes>,
}

impl Eligibility {
	pub fn snapshot(storage: &Storage, codex: &Codex) -> Result<Self, String> {
		Ok(Self {
			deregistering: invariants::deregistering(storage, codex)?,
			initial: codes(storage, codex)?,
			before_report: BTreeMap::new(),
		})
	}

	pub fn code(&self, report: usize, para: ParaId) -> Option<ValidationCodeHash> {
		self.before_report.get(&report).unwrap_or(&self.initial).get(&para).copied()
	}

	/// Usually the pre-invocation snapshot suffices. After a possible code change,
	/// execute a discarded prefix to read the code at the next report's entry.
	/// This preserves ordered upgrades, rejected messages and host write failures
	/// without duplicating the code-upgrade state machine in the head predicate.
	/// Only code hashes are taken from probes; heads are still replayed independently.
	pub fn prefixes(
		&mut self,
		storage: &Storage,
		items: &[AccumulateItem],
		frame: &Value,
		privileges: &jam_std_common::Privileges,
		codex: &Codex,
	) -> Result<(), String> {
		let results =
			field(frame, "lastStepWorkResults")?.as_array().ok_or("expected work results")?;
		let limits = gas::limits(frame, results.len())?;
		let end = if gas::before_checkpoint(frame)? {
			0
		} else {
			gas::interrupted(frame)?.unwrap_or(results.len())
		};
		let mut changed = false;
		let mut effective_index = 0;
		for (index, result) in results.iter().enumerate().take(end) {
			if limits[index] < gas::cost(result)? {
				continue;
			}
			if changed {
				// Include all arrivals: Accumulate processes them before any report.
				// Preserve the original reports' gas limits; omit the interrupted suffix.
				let prefix = items[..index]
					.iter()
					.chain(&items[results.len()..])
					.map(|item| {
						AccumulateItem::decode(&mut &item.encode()[..]).map_err(|e| e.to_string())
					})
					.collect::<Result<Vec<_>, _>>()?;
				let (_, next, _) = super::replay::accumulate_block_recovery(
					storage.clone(),
					prefix,
					bounded_integer(field(frame, "now")?, "now")?,
					privileges.clone(),
					None,
					None,
				)?;
				self.before_report.insert(effective_index, codes(&next, codex)?);
			}
			if let Some(messages) =
				result.pointer("/result/value/value/upwardMessages").and_then(Value::as_array)
			{
				for message in messages {
					let (tag, payload) = variant(message)?;
					changed |= tag == "ParachainSetValidationCode" ||
						tag == "ParachainCleanUp" ||
						(tag == "RequestCodeUpgrade" &&
							variant(field(payload, "phase")?)?.0 == "Apply");
				}
			}
			effective_index += 1;
		}
		Ok(())
	}
}
