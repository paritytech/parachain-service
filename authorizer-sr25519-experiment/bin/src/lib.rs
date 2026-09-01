//! EXPERIMENT — see `../README.md`. Blob builder for the sr25519 authorizer variant, a copy of
//! `authorizer/bin`.

pub const BLOB: &[u8] = jam_pvm_builder::pvm_binary!("parachain-authorizer-sr25519-experiment");
