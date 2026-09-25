//! The PVM blobs the parachain service's tests need.
//!
//! Each is compiled from its guest crate on demand by `cargo-jam-build`, at most once per
//! process, and the guest's own `[package.metadata.jam]` says how. Nothing is embedded at compile
//! time, so building this crate does not build a blob.

/// The service's JAM program blob.
pub fn blob() -> Vec<u8> {
	cargo_jam_build::blob("parachain-service")
}

/// Blake2b-256 hash of [`blob`] (its JAM code hash).
pub fn hash() -> [u8; 32] {
	cargo_jam_build::hash("parachain-service")
}

/// The mock transfer-destination service blob (gas benchmarks only) — a
/// realistic memo handler standing in for a legitimate `TransferOut`
/// destination.
#[cfg(feature = "test-utils")]
pub fn mock_dest_blob() -> Vec<u8> {
	cargo_jam_build::blob("mock-dest-service")
}

/// The ed25519 authorizer's JAM program blob, which the service's tests authorize with.
#[cfg(feature = "test-utils")]
pub fn authorizer_blob() -> Vec<u8> {
	cargo_jam_build::blob("parachain-authorizer-ed25519")
}

/// The frameless runtime's PVF: the linked program, not a JAM container, because that is what
/// the service resolves `jam_validate_block` out of and runs as a nested PVM. The crate declares
/// the `generic` blob type, which is what makes its blob that program.
#[cfg(feature = "test-utils")]
pub fn frameless_pvf() -> Vec<u8> {
	cargo_jam_build::blob("frameless")
}

#[cfg(feature = "test-utils")]
pub mod mock;
