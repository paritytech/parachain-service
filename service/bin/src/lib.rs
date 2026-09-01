//! PVM blob builder for the parachain service.
//!
//! The blob is compiled from the `parachain-service` guest crate by this crate's
//! `build.rs` (via `jam-pvm-builder`) and embedded here at compile time. Depend on
//! this crate to get the service blob without a separate `just build` step; a bare
//! `cargo test` rebuilds it automatically when the guest source changes.

/// The service's JAM program blob.
pub const BLOB: &[u8] = jam_pvm_builder::pvm_binary!("parachain-service");

/// Blake2b-256 hash of [`BLOB`] (its JAM code hash).
pub const HASH: [u8; 32] = *jam_pvm_builder::pvm_binary_hash!("parachain-service");

/// The mock transfer-destination service blob (gas benchmarks only) — a
/// realistic memo handler standing in for a legitimate `TransferOut`
/// destination, to size `MAX_TRANSFER_GAS`.
#[cfg(feature = "test-utils")]
pub const MOCK_DEST_BLOB: &[u8] = jam_pvm_builder::pvm_binary!("mock-dest-service");

#[cfg(feature = "test-utils")]
pub mod mock;
