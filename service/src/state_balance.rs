//! State-balance accounting (spec §6.1, DECISIONS.md D-3).
//!
//! JAM bills the service for everything it holds in state; the service
//! re-attributes that per parachain via `used_state_balance`, using the "as if
//! sole user" rule: each referencer pays the full encoded cost of a
//! `(key, value)` pair containing only its own contribution.
//!
//! All balances are `u64` (D-3), so the worst-case compact-encoded balance is
//! 9 B, not the design's 17 B (`Compact<u128>`) — the derived constants below
//! differ from the §6.1 tables accordingly and are unit-tested at the bottom.
//! TODO: needs upstreaming into the §6.1 tables.

use crate::{
	constants::{CORE_COUNT, MAX_INCOMING_TRANSFERS, MAX_STAGED_VALIDATOR_KEYS},
	hashing::blake2_256,
	state::{
		kv::KeyValueStorage,
		log::{AccumulateLog, InsufficientBalanceReason},
		para_info::{ParaInfo, Parachains},
		preimage_registry::PreimageRegistry,
	},
};
use codec::{Compact, Encode};
use jam_pvm_common::accumulate::{forget, query, solicit, LookupRequestStatus};
use parachain_service_interface::types::{Balance, Hash, ParaId, Timeslot, ASSET_HUB_PARA_ID};

/// Gray Paper `C_itemdeposit`.
pub const ITEM_DEPOSIT: u64 = 10;
/// Gray Paper per-entry octet overhead (`C_bytedeposit` applies per octet;
/// each storage entry carries 34 overhead octets).
pub const ENTRY_OVERHEAD: u64 = 34;

/// Byte width of a SCALE compact-encoded integer.
pub fn compact_len(n: u64) -> u64 {
	Compact(n).encoded_size() as u64
}

/// The single-referencer footprint a parachain pays for a solicited preimage of
/// length `len` (§6.1): the JAM preimage request (2 items = 20, octets 81 + len)
/// plus the sole-user `preimage_registry` entry (1 item = 10, octets 34 + 5
/// (singleton `{ParaId}` set) + 37 (1 B tag + 32 B hash + 4 B len)).
/// Total: `187 + len` balance units.
pub fn preimage_footprint(len: u32) -> Balance {
	(20 + 81 + len as u64) + (10 + ENTRY_OVERHEAD + 5 + 37)
}

/// Per-entry footprint of a `key_value_storage` write (§6.1):
/// item 10 + overhead 34 + value (compact len + bytes) + key (1 B tag + 4 B
/// ParaId + compact len + bytes) = `49 + compactLen(k) + k + compactLen(v) + v`.
pub fn kv_entry_footprint(key_len: usize, value_len: usize) -> Balance {
	let (k, v) = (key_len as u64, value_len as u64);
	ITEM_DEPOSIT + ENTRY_OVERHEAD + 5 + compact_len(k) + k + compact_len(v) + v
}

/// SCALE size of a `Vec<u8>` value: compact length prefix + bytes.
fn vec_bytes(len: usize) -> u64 {
	compact_len(len as u64) + len as u64
}

/// Worst-case octets of the `(ParaId, ParaInfo)` entry (§6.1 table, with 9 B
/// worst-case `Compact<u64>` balances per D-3): overhead 34 + tag 1 + key 4 +
/// head 4 098 + validation_code 38 + pending_upgrade 42 + balances 9 + 9 +
/// is_deregistering 1, plus 1 item.
pub const PARA_INFO_FOOTPRINT: Balance =
	ENTRY_OVERHEAD + 1 + 4 + (2 + 4096) + 38 + 42 + 9 + 9 + 1 + ITEM_DEPOSIT;

/// The flat `parachain_log` reserve (§6.1 table): overhead 34 + key 5 +
/// 64 KiB value cap, plus 1 item.
pub const PARA_LOG_FOOTPRINT: Balance = ENTRY_OVERHEAD + 5 + 65536 + ITEM_DEPOSIT;

/// Per-para baseline footprint: the worst-case state cost of an empty parachain.
pub const BASELINE_FOOTPRINT: Balance = PARA_INFO_FOOTPRINT + PARA_LOG_FOOTPRINT;

/// Value octets of one queued transfer (§6.1, with `Compact<u64>` amount per
/// D-3): ServiceId 4 + amount 9 (worst case) + memo 128.
pub const INCOMING_TRANSFER_VALUE_OCTETS: u64 = 4 + 9 + 128;

/// Full balance-unit cost of one worst-case `incoming_transfers` bucket — a
/// bucket holding a single transfer (maximal fragmentation): 1 item + JAM
/// overhead 34 + key (1 B tag + 4 B slot) + `Vec` prefix 1 + the transfer +
/// `Some(next_slot)` 5.
pub const INCOMING_TRANSFER_ENTRY_FOOTPRINT: Balance =
	ITEM_DEPOSIT + ENTRY_OVERHEAD + 5 + 1 + INCOMING_TRANSFER_VALUE_OCTETS + 5;

/// §5.1: what Asset Hub is charged for the unreserved part of the transfer
/// queue, priced per worst-case bucket as §6.1 sizes the reservation itself.
pub fn excess_transfer_footprint(count: u64) -> Balance {
	if count <= MAX_INCOMING_TRANSFERS as u64 {
		0
	} else {
		(count - MAX_INCOMING_TRANSFERS as u64) * INCOMING_TRANSFER_ENTRY_FOOTPRINT
	}
}

/// §5.1: apply the change in that charge after the queue's `count` moved from
/// `old_count` to `new_count`. The transfer funds the entry, so Asset Hub's
/// `used_state_balance` and `total_state_balance` move together by the
/// per-bucket cost and nothing else — the available balance is unchanged by
/// queue admission and draining, so replay order does not matter. Admission and
/// draining both go through here, so the two cannot drift.
pub fn reattribute_transfer_queue(old_count: u64, new_count: u64) {
	let delta = excess_transfer_footprint(new_count) as i128 -
		excess_transfer_footprint(old_count) as i128;
	if delta == 0 {
		return;
	}
	let Some(mut pi) = Parachains::get(ASSET_HUB_PARA_ID) else { return };
	if delta > 0 {
		pi.used_state_balance = pi.used_state_balance.saturating_add(delta as Balance);
		pi.total_state_balance = pi.total_state_balance.saturating_add(delta as Balance);
	} else {
		pi.used_state_balance = pi.used_state_balance.saturating_sub((-delta) as Balance);
		pi.total_state_balance = pi.total_state_balance.saturating_sub((-delta) as Balance);
	}
	Parachains::set(ASSET_HUB_PARA_ID, &pi);
}

/// Asset Hub's service-global reservation (§6.1 "Asset Hub baseline footprint"),
/// pre-provisioned at registration rather than charged as the items grow.
pub const ASSET_HUB_GLOBAL_ITEMS_FOOTPRINT: Balance = {
	// staged_validator_keys: 34 + 1 (key) + 2 (compact len) + 1023 * 336, 1 item.
	let staged_keys = ENTRY_OVERHEAD + 1 + 2 + (MAX_STAGED_VALIDATOR_KEYS as u64) * 336;
	// pending_assigns: per core 34 + 3 (tag + u16 CoreIndex key) + 2 (compact len)
	// + the authorizer queue + 5 (Option<ServiceId>).
	let pending_assigns = (CORE_COUNT as u64) * (ENTRY_OVERHEAD + 3 + 2 + 79 * 32 + 5);
	// pending_assign_cores: 34 + 1 + 2 + 341 * (u16 core 2 + slot 4), 1 item.
	let pending_assign_cores = ENTRY_OVERHEAD + 1 + 2 + (CORE_COUNT as u64) * 6;
	// incoming_transfer_chain: 34 + 1 (key) + 1 (Option tag) + first 4 + last 4
	// + count 4 (the counter added per SPEC_GAPS #2; see state::transfers).
	let transfer_chain = ENTRY_OVERHEAD + 1 + 1 + 4 + 4 + 4;
	// Fixed storage items: the two singletons, the chain pointer, one per core.
	let fixed_items = (3 + CORE_COUNT as u64) * ITEM_DEPOSIT;
	staged_keys +
		pending_assigns +
		pending_assign_cores +
		transfer_chain +
		fixed_items +
		(MAX_INCOMING_TRANSFERS as u64) * INCOMING_TRANSFER_ENTRY_FOOTPRINT
};

/// Registration baseline for a para: Asset Hub reserves its service-global items
/// on top of the generic baseline (§6.1).
pub fn baseline_for(para_id: ParaId) -> Balance {
	if para_id == ASSET_HUB_PARA_ID {
		BASELINE_FOOTPRINT + ASSET_HUB_GLOBAL_ITEMS_FOOTPRINT
	} else {
		BASELINE_FOOTPRINT
	}
}

// --- Preimage-registry referencer multiplexing (§6.1) ---------------------

/// Outcome of a `remove_referencer`.
pub struct RemoveOutcome {
	/// `true` iff the para was kept as a referencer awaiting the second,
	/// expunging forget (§6.1 two-step release).
	pub retained: bool,
	/// Log entry to append (`ForgetAgainAt`), if any.
	pub log: Option<AccumulateLog>,
}

/// Add `para_id` as a referencer of `(hash, len)`, charging its footprint and
/// forwarding JAM `solicit` on the empty→non-empty transition or on a rescue
/// (§6.1). Idempotent. Returns the `InsufficientStateBalance` log on rejection.
pub fn add_referencer(para_id: ParaId, hash: &Hash, len: u32) -> Result<(), AccumulateLog> {
	let status = query(hash, len as usize);
	// A `solicit` while the request is `Unrequested` RESCUES the still-held blob
	// back to available; the original unrequest slot still gates the next forget.
	let is_rescue = matches!(status, Some(LookupRequestStatus::Unrequested { .. }));
	let mut entry = PreimageRegistry::get(hash, len).unwrap_or_default();

	if entry.referencers.contains(&para_id) {
		// Already a referencer — idempotent, no extra balance. But if the request
		// is in limbo, the service still must fire JAM `solicit` to rescue it.
		if is_rescue {
			jam_solicit(hash, len);
		}
		return Ok(());
	}

	let delta = preimage_footprint(len);
	let mut pi = Parachains::get(para_id).expect("caller checked the para is live; qed");
	if !pi.has_headroom(delta) {
		return Err(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::Solicit { hash: *hash, len: len.into() },
		});
	}

	let was_empty = entry.referencers.is_empty();
	if is_rescue {
		// The existing referencers are only retained ghosts from the two-step
		// forget (`Unrequested` has no live referencer). The soliciting para
		// becomes the sole LIVE referencer: refund and drop every ghost, else
		// one JAM request entry would be double-counted (§6.1).
		for ghost in core::mem::take(&mut entry.referencers) {
			if ghost == para_id {
				continue;
			}
			if let Some(mut ghost_pi) = Parachains::get(ghost) {
				ghost_pi.refund(delta);
				Parachains::set(ghost, &ghost_pi);
			}
		}
	}

	entry.referencers.insert(para_id);
	PreimageRegistry::set(hash, len, &entry);
	pi.charge(delta);
	Parachains::set(para_id, &pi);

	if was_empty || is_rescue {
		jam_solicit(hash, len);
	}
	Ok(())
}

/// Release `para_id`'s reference on `(hash, len)` via JAM's two-step `forget`
/// (§6.1). Idempotent: not a referencer is a no-op.
pub fn remove_referencer(para_id: ParaId, hash: &Hash, len: u32, now: Timeslot) -> RemoveOutcome {
	if !PreimageRegistry::has_referencer(hash, len, para_id) {
		return RemoveOutcome { retained: false, log: None };
	}
	let mut entry = PreimageRegistry::get(hash, len).expect("has_referencer checked; qed");
	let delta = preimage_footprint(len);

	if entry.referencers.len() > 1 {
		// A non-last referencer leaves immediately: the rest still cover the
		// live JAM request. Refund now, no JAM forget.
		entry.referencers.remove(&para_id);
		PreimageRegistry::set(hash, len, &entry);
		refund_para(para_id, delta);
		return RemoveOutcome { retained: false, log: None };
	}

	// Last referencer leaves — JAM `forget` behaviour depends on the request
	// status. `forget_implication` mirrors JAM's own gating, so the service
	// never issues a `forget` JAM would reject.
	let Some(status) = query(hash, len as usize) else {
		// Registry says referenced but JAM knows no request: bookkeeping
		// desync. Drop our side and refund; nothing to forget.
		// FIXME: consensus-critical — should be impossible (SPEC_GAPS #4).
		expunge_entry(para_id, hash, len, delta);
		return RemoveOutcome { retained: false, log: None };
	};

	use jam_pvm_common::accumulate::ForgetImplication as F;
	match status.forget_implication(now) {
		F::Drop | F::Expunge => {
			// Never provided (Drop) or past the turnaround (Expunge): the JAM
			// `forget` removes the request outright; deposit lifted now.
			jam_forget(hash, len);
			expunge_entry(para_id, hash, len, delta);
			RemoveOutcome { retained: false, log: None }
		},
		F::Unrequest => {
			// Available (or rescued past its gate): the first `forget` only
			// unrequests. Keep the referencer (still charged), log when the
			// expunging second forget becomes due.
			jam_forget(hash, len);
			let due = query(hash, len as usize)
				.map(|s| match s.forget_implication(now) {
					F::NotYetExpunge { success_after } => success_after,
					_ => now + crate::constants::EXPUNGE_PERIOD,
				})
				.unwrap_or(now + crate::constants::EXPUNGE_PERIOD);
			RemoveOutcome {
				retained: true,
				log: Some(AccumulateLog::ForgetAgainAt { hash: *hash, len: len.into(), due }),
			}
		},
		F::NotYetUnrequest { success_after } | F::NotYetExpunge { success_after } => {
			// Too early: no JAM call; the caller retries strictly after `due`.
			RemoveOutcome {
				retained: true,
				log: Some(AccumulateLog::ForgetAgainAt {
					hash: *hash,
					len: len.into(),
					due: success_after,
				}),
			}
		},
	}
}

fn expunge_entry(para_id: ParaId, hash: &Hash, len: u32, delta: Balance) {
	PreimageRegistry::remove(hash, len);
	refund_para(para_id, delta);
}

fn refund_para(para_id: ParaId, delta: Balance) {
	if let Some(mut pi) = Parachains::get(para_id) {
		pi.refund(delta);
		Parachains::set(para_id, &pi);
	}
}

fn jam_solicit(hash: &Hash, len: u32) {
	// FIXME: consensus-critical — a JAM-level failure here (e.g. FULL: the real
	// service balance cannot cover the request) means the internal accounting
	// diverged from JAM's (SPEC_GAPS #4). Panic reverts to the last checkpoint.
	solicit(hash, len as usize).expect("internal accounting covers the solicit; qed");
}

fn jam_forget(hash: &Hash, len: u32) {
	forget(hash, len as usize).expect("forget gated on forget_implication; qed");
}

// --- Parachain KV accounting (§6.1) ----------------------------------------

/// Replay a `SetKV`: upsert `key_value_storage[(para_id, key)]`, delta-charged
/// per §6.1. Rejected (state unchanged, `FromSetKV` logged) when a positive
/// delta exceeds headroom.
pub fn apply_set_kv(para_id: ParaId, key: &[u8], value: &[u8]) -> Result<(), AccumulateLog> {
	let mut pi = Parachains::get(para_id).expect("caller checked the para is live; qed");
	let old = KeyValueStorage::get(para_id, key);
	// Signed delta in balance units: a fresh entry pays the full footprint, an
	// overwrite pays only the value-size difference (§6.1).
	let delta: i128 = match &old {
		None => kv_entry_footprint(key.len(), value.len()) as i128,
		Some(old_v) => vec_bytes(value.len()) as i128 - vec_bytes(old_v.len()) as i128,
	};
	if delta > 0 && !pi.has_headroom(delta as Balance) {
		return Err(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::SetKV { key_hash: blake2_256(key) },
		});
	}
	if delta >= 0 {
		pi.charge(delta as Balance);
	} else {
		pi.refund((-delta) as Balance);
	}
	Parachains::set(para_id, &pi);
	KeyValueStorage::set(para_id, key, value);
	Ok(())
}

/// Replay a `RemoveKV`: drop the entry and refund its footprint. No-op on
/// absent keys.
pub fn apply_remove_kv(para_id: ParaId, key: &[u8]) {
	let Some(value) = KeyValueStorage::get(para_id, key) else { return };
	refund_para(para_id, kv_entry_footprint(key.len(), value.len()));
	KeyValueStorage::remove(para_id, key);
}

/// The exact `used_state_balance` a para may hold at clean-up (§6.4): its
/// baseline plus the footprints of its active and (if any) pending validation
/// code — nothing else.
pub fn clean_up_allowed_balance(pi: &ParaInfo, para_id: ParaId) -> Balance {
	let active = pi.validation_code.as_ref().map_or(0, |vc| preimage_footprint(vc.code_ref.len));
	let pending = pi
		.pending_upgrade
		.as_ref()
		.map_or(0, |(vc, _)| preimage_footprint(vc.code_ref.len));
	baseline_for(para_id) + active + pending
}

// Re-exported for the transfer-admission rule (§5.1).
pub use self::admission::*;
mod admission {
	use super::*;

	/// §5.1: a transfer past the reserved portion is recorded only if its amount
	/// covers the cost of its own worst-case queue entry.
	pub fn transfer_covers_own_slot(amount: Balance) -> bool {
		amount >= INCOMING_TRANSFER_ENTRY_FOOTPRINT
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn preimage_footprint_works() {
		// §6.1: 187 + len.
		assert_eq!(preimage_footprint(0), 187);
		assert_eq!(preimage_footprint(1000), 1187);
	}

	#[test]
	fn kv_entry_footprint_works() {
		// §6.1: 49 + compactLen(k) + k + compactLen(v) + v.
		assert_eq!(kv_entry_footprint(3, 5), 49 + 1 + 3 + 1 + 5);
		assert_eq!(kv_entry_footprint(100, 20_000), 49 + 2 + 100 + 4 + 20_000);
	}

	#[test]
	fn baseline_footprint_works() {
		// The design's table says 69 847 with 17 B `Compact<u128>` balances;
		// with 9 B `Compact<u64>` (D-3) both balance fields shrink by 8 B.
		assert_eq!(PARA_INFO_FOOTPRINT, 4_262 - 16);
		assert_eq!(PARA_LOG_FOOTPRINT, 65_585);
		assert_eq!(BASELINE_FOOTPRINT, 69_847 - 16);
	}

	#[test]
	fn asset_hub_footprint_works() {
		// §6.1 says 1 226 388 fixed + 204 × N with 17 B `Compact<u128>` amounts;
		// with 9 B `Compact<u64>` (D-3) a bucket costs 196 instead. The fixed
		// part is unaffected by the balance width and must match exactly.
		assert_eq!(INCOMING_TRANSFER_ENTRY_FOOTPRINT, 196);
		assert_eq!(
			ASSET_HUB_GLOBAL_ITEMS_FOOTPRINT,
			1_226_388 + (MAX_INCOMING_TRANSFERS as u64) * 196
		);
	}

	#[test]
	fn compact_len_works() {
		assert_eq!(compact_len(0), 1);
		assert_eq!(compact_len(63), 1);
		assert_eq!(compact_len(64), 2);
		assert_eq!(compact_len(16383), 2);
		assert_eq!(compact_len(16384), 4);
		assert_eq!(compact_len((1 << 30) - 1), 4);
		assert_eq!(compact_len(1 << 30), 5);
		assert_eq!(compact_len(u64::MAX), 9);
	}
}
