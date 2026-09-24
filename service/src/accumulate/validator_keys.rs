//! Chunked JAM `designate` flow (spec §5.3).
//!
//! A full staging set (up to 1023 × 336 B) cannot fit one work-report, so Asset
//! Hub emits `SetValidatorKeys { keys, is_last }` chunks across blocks; the
//! service buffers them in `staged_validator_keys` and finalizes on `is_last`.
//! The buffer's worst case is pre-provisioned in Asset Hub's baseline (§6.1),
//! so partial appends charge no balance.

use crate::{
	constants::MAX_STAGED_VALIDATOR_KEYS,
	state::{
		log::{AccumulateLog, InsufficientBalanceReason},
		validator_keys::{StagedKeys, StagedValidatorKeys},
	},
};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::designate;
use jam_types::{OpaqueValKeyset, OpaqueValKeysets};
use parachain_service_core::types::ValidatorKey;

/// Replay of `SetValidatorKeys { keys, is_last }` (Asset Hub only, §4.3).
pub fn apply(chunk: Vec<ValidatorKey>, is_last: bool, logs: &mut Vec<AccumulateLog>) {
	let staged = StagedValidatorKeys::get();

	if is_last {
		// Final chunk: clear the buffer either way. An empty chunk is Asset Hub's
		// abort path, which discards the staged keys without calling `designate`.
		StagedValidatorKeys::clear();
		if chunk.is_empty() {
			return;
		}

		// Assemble in memory and hand to `designate`, which rejects a length
		// outside `valcount` or a caller that is not the delegator. The final
		// chunk never persists, so no headroom check.
		let assembled: Vec<OpaqueValKeyset> =
			staged.iter().chain(chunk.iter()).map(|raw| decode_key(raw)).collect();
		let len = assembled.len();
		let designated =
			OpaqueValKeysets::try_from(assembled).is_ok_and(|set| designate(&set).is_ok());
		if !designated {
			logs.push(AccumulateLog::DesignateRejected { len: (len as u32).into() });
		}
		return;
	}

	// Partial append: the chunk stays in the buffer until finalization.
	if staged.len() + chunk.len() > MAX_STAGED_VALIDATOR_KEYS {
		logs.push(AccumulateLog::StagedValidatorKeysOverflow);
		return;
	}
	let mut staged: StagedKeys = staged;
	for key in chunk {
		staged.try_push(key).expect("length checked against the bound above; qed");
	}
	// Appending grows the (baseline-covered) buffer; a backstop write failure
	// (§6.1 invariant) logs the rejection and the chunk is dropped
	// — accumulate continues.
	if StagedValidatorKeys::set(&staged).is_err() {
		logs.push(AccumulateLog::InsufficientStateBalance {
			reason: InsufficientBalanceReason::StagedValidatorKeys,
		});
	}
}

/// A [`ValidatorKey`] is the 336-byte concatenation of the `OpaqueValKeyset`
/// fields (bandersnatch 32 ‖ ed25519 32 ‖ bls 144 ‖ metadata 128).
fn decode_key(raw: &ValidatorKey) -> OpaqueValKeyset {
	jam_codec::Decode::decode(&mut &raw[..])
		.expect("OpaqueValKeyset is exactly 336 fixed bytes; qed")
}
