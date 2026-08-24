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
		log::AccumulateLog,
		validator_keys::{StagedKeys, StagedValidatorKeys},
	},
};
use alloc::vec::Vec;
use jam_pvm_common::accumulate::designate;
use jam_types::{OpaqueValKeyset, OpaqueValKeysets};
use parachain_service_interface::types::ValidatorKey;

/// Replay of `SetValidatorKeys { keys, is_last }` (Asset Hub only, §4.3).
pub fn apply(chunk: Vec<ValidatorKey>, is_last: bool, logs: &mut Vec<AccumulateLog>) {
	let staged = StagedValidatorKeys::get();

	if is_last {
		// Final chunk: assemble in memory, hand to `designate`, clear the buffer
		// either way. The final chunk never persists, so no headroom check.
		let assembled: Vec<OpaqueValKeyset> =
			staged.iter().chain(chunk.iter()).map(|raw| decode_key(raw)).collect();
		let len = assembled.len();
		StagedValidatorKeys::clear();

		// JAM `designate` accepts only the protocol's exact validator count;
		// `OpaqueValKeysets` (a FixedVec) enforces it. A wrong length rejects
		// the set — this is also Asset Hub's abort path (empty set + is_last).
		match OpaqueValKeysets::try_from(assembled) {
			Ok(set) => {
				if designate(&set).is_err() {
					// TODO: no log is specified for a JAM-level designate
					// failure (e.g. the service is not the delegator);
					// reuse DesignateRejected. Needs upstreaming.
					logs.push(AccumulateLog::DesignateRejected { len: (len as u32).into() });
				}
			},
			Err(_) => logs.push(AccumulateLog::DesignateRejected { len: (len as u32).into() }),
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
	StagedValidatorKeys::set(&staged);
}

/// A [`ValidatorKey`] is the 336-byte concatenation of the `OpaqueValKeyset`
/// fields (bandersnatch 32 ‖ ed25519 32 ‖ bls 144 ‖ metadata 128).
fn decode_key(raw: &ValidatorKey) -> OpaqueValKeyset {
	jam_codec::Decode::decode(&mut &raw[..])
		.expect("OpaqueValKeyset is exactly 336 fixed bytes; qed")
}
