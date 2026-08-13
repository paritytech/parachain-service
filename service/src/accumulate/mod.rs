//! `accumulate` entry point of the parachain service (spec §5.1).
//!
//! The work runs in three phases, in order: all always-accumulate work first
//! (due authorizer-queue flushes, then incoming-transfer processing) and then
//! per-work-package work. Because selected work-reports are not replayed
//! automatically, the service checkpoints after finishing each work-report so
//! progress survives a later out-of-gas or panic in the same invocation.

pub mod assigns;
pub mod code_upgrades;
pub mod management;
pub mod package;
pub mod transfers;
pub mod upward;
pub mod validator_keys;

use jam_pvm_common::accumulate::{accumulate_items, checkpoint};
use jam_types::{AccumulateItem, Hash, ServiceId, Slot};

#[derive(Debug)]
pub enum AccumulateError {}

pub fn accumulate(
	now: Slot,
	service_id: ServiceId,
	_item_count: usize,
) -> Result<Option<Hash>, AccumulateError> {
	let items = accumulate_items();

	// Phase 1: always-accumulate — flush due authorizer-queue assigns (§5.1).
	assigns::apply_due_assigns(now, service_id);

	// Phase 2: incoming-transfer processing (§5.1). JAM already credited the
	// balance unconditionally; recording is best effort.
	for item in &items {
		if let AccumulateItem::Transfer(transfer) = item {
			transfers::record_incoming(now, transfer);
		}
	}

	// The always-accumulate work is done; protect it from a later failure.
	// TODO: the design only mandates checkpointing after each work-report; the
	// phase boundary here is an extra safety point.
	checkpoint();

	// Phase 3: per-work-package work, in operand order (§5.1 steps 1–7).
	for item in items {
		if let AccumulateItem::WorkItem(record) = item {
			package::process(now, service_id, &record);
			// §5.1: checkpoint after each work-report so its effects survive a
			// later out-of-gas or panic (SPEC_GAPS #3).
			checkpoint();
		}
	}

	Ok(None)
}
