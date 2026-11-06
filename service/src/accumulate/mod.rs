//! `accumulate` entry point of the parachain service (spec §5.1).
//!
//! The work runs in three phases, in order: due authorizer-queue flushes,
//! incoming-transfer processing, and per-work-package work. Each phase is paid
//! from its own gas: the due assigns from the always-accumulate allowance, each
//! transfer from the gas it carries, and each work-report from the gas it
//! declared itself. The service checkpoints after the flush, and after applying a
//! report it yields the head commitment so far and checkpoints, so running the
//! budget dry later cannot undo either. Due assigns that did not fit
//! the allowance are applied at the end if gas is left, and are otherwise
//! carried over to the next block.

pub mod assigns;
pub mod code_upgrades;
pub mod foreign_services;
pub mod management;
pub mod package;
pub mod transfers;
pub mod upward;
pub mod validator_keys;

use crate::{head_commitment::HeadTracker, state::log::ParachainLogs};
use jam_pvm_common::accumulate::{accumulate_items, checkpoint, gas, yield_hash};
use jam_types::{AccumulateItem, Hash, ServiceId, Slot, UnsignedGas};
use parachain_service_core::types::CORETIME_PARA_ID;

#[derive(Debug)]
pub enum AccumulateError {}

pub fn accumulate(
	now: Slot,
	service_id: ServiceId,
	_item_count: usize,
) -> Result<Option<Hash>, AccumulateError> {
	let items = accumulate_items();
	// §5.5: every head this block moves is diffed against its pre-block value to
	// build the commitment `accumulate` returns.
	let mut heads = HeadTracker::new();

	// JAM's gas for this invocation is the always-accumulate allowance plus the
	// gas each work-report and incoming transfer brought along, so whatever the
	// items did not bring is the allowance.
	let declared = items.iter().fold(0 as UnsignedGas, |sum, item| {
		sum.saturating_add(match item {
			AccumulateItem::WorkItem(record) => record.gas_limit,
			AccumulateItem::Transfer(transfer) => transfer.gas_limit,
		})
	});
	let allowance = gas().saturating_sub(declared);

	// Phase 1: always-accumulate — flush the due authorizer-queue assigns the
	// allowance pays for, then checkpoint so running out of gas later cannot undo
	// the flush (§5.1).
	flush_due_assigns(now, service_id, assigns::budget(allowance));
	checkpoint();

	// Phase 2: incoming-transfer processing (§5.1). JAM already credited the
	// balances unconditionally; recording is best effort. Transfers are
	// written once per whole bucket, never per transfer (D-8).
	let transfers: alloc::vec::Vec<_> = items
		.iter()
		.filter_map(|item| match item {
			AccumulateItem::Transfer(transfer) => Some(transfer),
			_ => None,
		})
		.collect();
	transfers::record_incoming(&transfers);

	// TODO: the design only mandates checkpointing after the flush and after each
	// work-report; the end of transfer recording is an extra safety point.
	if !transfers.is_empty() {
		checkpoint();
	}

	// Phase 3: per-work-package work, in operand order (§5.1 steps 1–6).
	for item in items {
		if let AccumulateItem::WorkItem(record) = item {
			// §5.1: a report that cleared the gas gate yields the commitment over the
			// heads changed so far (§5.5), then checkpoints, so running out of gas or
			// panicking later cannot undo either.
			if package::process(now, service_id, &record, &mut heads) {
				if let Some(root) = heads.root() {
					yield_hash(&root);
				}
				checkpoint();
			}
		}
	}

	// The due assigns still pending are applied while the gas left pays for
	// them (§5.1). They touch no head, so the commitment is final already.
	let commitment = heads.root();
	flush_due_assigns(now, service_id, assigns::budget(gas()));

	Ok(commitment)
}

/// Fire up to `budget` due assigns. `AssignCore` is Coretime only, so the
/// rejections are logged against it.
fn flush_due_assigns(now: Slot, service_id: ServiceId, budget: usize) {
	let logs = assigns::apply_due_assigns(now, service_id, budget);
	ParachainLogs::append_accumulate(CORETIME_PARA_ID, now, logs);
}
