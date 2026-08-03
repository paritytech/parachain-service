#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;

use alloc::format;
use codec::Encode;
use jam_pvm_common::{declare_service, Service};
use jam_types::{
	CoreIndex, Hash, ServiceId, Slot, WorkOutput as WorkResult, WorkPackageHash, WorkPayload,
};

mod accumulate;
pub mod refine;
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

		WorkResult(digest.encode())
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
