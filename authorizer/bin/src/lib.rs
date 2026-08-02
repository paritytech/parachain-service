//! PVM blob builder for the parachain authorizer.
//!
//! The blob is compiled from the `parachain-authorizer` guest crate by this
//! crate's `build.rs` (via `jam-pvm-builder`) and embedded here at compile time.
//! Depend on this crate to get the authorizer blob without a separate build step.

/// The authorizer's JAM program blob.
pub const BLOB: &[u8] = jam_pvm_builder::pvm_binary!("parachain-authorizer");

/// Blake2b-256 hash of [`BLOB`] (its JAM code hash).
pub const HASH: [u8; 32] = *jam_pvm_builder::pvm_binary_hash!("parachain-authorizer");
