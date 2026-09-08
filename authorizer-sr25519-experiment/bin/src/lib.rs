//! EXPERIMENT — see `../README.md`. Blob accessor for the sr25519 authorizer variant.

/// The experimental authorizer's JAM program blob.
pub fn blob() -> Vec<u8> {
	cargo_jam_build::blob("parachain-authorizer-sr25519-experiment")
}

/// The ed25519 authorizer's, which `tests/gas.rs` measures this one against.
pub fn ed25519_blob() -> Vec<u8> {
	cargo_jam_build::blob("parachain-authorizer-ed25519")
}
