//! Native executors for the parachain service and parachain runtime blobs.
//!
//! The crate has no default features so joining the main workspace does not pull
//! either executor dependency tree into normal builds. Consumers opt into only
//! the backend they need.

#[cfg(feature = "service")]
pub mod host;

#[cfg(feature = "jam")]
pub mod polkajam;
#[cfg(feature = "jam")]
pub use polkajam as pj;

#[cfg(feature = "runtime")]
pub mod runtime;
#[cfg(feature = "service")]
pub mod service;
