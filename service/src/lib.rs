#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

extern crate alloc;
use jam_pvm_common::*;
use jam_types::{WorkOutput as WorkResult, *};

mod accumulate;
mod refine;
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
        refine::refine(core_index, item_index, service_id, payload, package_hash)
    }

    fn accumulate(slot: Slot, id: ServiceId, item_count: usize) -> Option<Hash> {
        accumulate::accumulate(slot, id, item_count)
    }
}
