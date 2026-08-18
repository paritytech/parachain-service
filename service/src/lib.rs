#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;

use alloc::format;
use codec::Encode;
use jam_pvm_common::{declare_service, Service};
use jam_types::{
	CoreIndex, Hash, ServiceId, Slot, WorkOutput as WorkResult, WorkPackageHash, WorkPayload,
};
use work_digest::{ParachainWorkDigest, RefineLog, MAX_REFINE_OUTPUT_SIZE};

pub mod accumulate;
pub mod constants;
pub mod hashing;
pub mod head_commitment;
pub mod pvf;
pub mod refine;
pub mod state;
pub mod state_balance;
pub mod work_digest;

/// Directory of this crate's `Cargo.toml`, used by `parachain-service-bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

pub struct ParachainService;
declare_service!(ParachainService);

impl Service for ParachainService {
	fn refine(
		core_index: CoreIndex,
		item_index: usize,
		service_id: ServiceId,
		payload: WorkPayload,
		package_hash: WorkPackageHash,
	) -> WorkResult {
		let digest = refine::refine(core_index, item_index, service_id, payload, package_hash);
		let encoded = digest.encode();

		// NOTE: since we only have a single work item, we can conveniently check the size here.
		let total_len = encoded.len().saturating_add(jam_pvm_common::refine::auth_trace().len());

		if total_len > MAX_REFINE_OUTPUT_SIZE {
			WorkResult(
				ParachainWorkDigest::Err {
					para_id: digest.para_id(),
					error: RefineLog::RefineOutputTooLarge,
				}
				.encode(),
			)
		} else {
			WorkResult(encoded)
		}
	}

	fn accumulate(slot: Slot, id: ServiceId, item_count: usize) -> Option<Hash> {
		match accumulate::accumulate(slot, id, item_count) {
			Ok(r) => r,
			Err(e) => {
				let msg = format!("BUG: Parachain Service accumulate crashed: {e:?}");

				jam_pvm_common::error!("{msg}");
				panic!("{msg}");
			},
		}
	}
}
