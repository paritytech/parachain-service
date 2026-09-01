//! PVM blob builder for the parasim service, mirroring `service/bin`.
//!
//! The blob is compiled from the `parasim-service` guest crate by this crate's
//! `build.rs` (via `pvm-builder`) and embedded here at compile time. Depend on
//! this crate to get the parasim blob (e.g. the mock sender embeds it for
//! byte-exact code hashes).

/// The parasim service's JAM program blob.
pub const BLOB: &[u8] = jam_pvm_builder::pvm_binary!("parasim-service");

/// Blake2b-256 hash of [`BLOB`] (its JAM code hash).
pub const HASH: [u8; 32] = *jam_pvm_builder::pvm_binary_hash!("parasim-service");