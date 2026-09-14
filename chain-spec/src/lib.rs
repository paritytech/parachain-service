//! Builder API for the parachain service's entry in a JAM chain spec.
//!
//! [`ParachainServiceSpec`] describes the service at genesis — the registered
//! parachains, each para's validation code and authorizer — and builds a
//! [`BuiltParachainService`] whose parts a caller places into a
//! [`ChainSpecConfig`](jam_chainspec::ChainSpecConfig) under the service's id.
//! The service's storage carries the per-parachain records and the preimage
//! registry in exactly the layout [`parachain_service`] writes (§3.1), the
//! validation-code and authorizer blobs are hosted as preimages of the service,
//! and the authorizer hashes come back from
//! [`ParachainServiceSpec::authorizer_hashes`] to fill the cores' queues.
//!
//! ```
//! use parachain_authorizer::aura::AuthConfig;
//! use parachain_chain_spec::{ParachainServiceSpec, ParachainSpec};
//! use parachain_service_core::types::ParaId;
//! use primitive_types::H256;
//!
//! let config = AuthConfig {
//!     para_ids: vec![ParaId(2000)],
//!     parachain_service: 1,
//!     collator_set_root: H256::zero(),
//!     collator_set_size: 1,
//!     slot_duration: 6,
//! };
//! let spec = ParachainServiceSpec::new(1, b"service code").parachain(
//!     ParachainSpec::new(ParaId(2000))
//!         .validation_code(b"verifier")
//!         .authorizer(b"authorizer blob", &config),
//! );
//! let hashes = spec.authorizer_hashes();
//! let genesis = spec.build()?;
//! assert_eq!(hashes.len(), 1);
//! assert!(genesis.preimages.iter().any(|b| b.as_slice() == b"authorizer blob"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod parachain;
mod service_spec;
#[cfg(test)]
mod tests;

pub use error::Error;
pub use parachain::ParachainSpec;
pub use service_spec::{BuiltParachainService, ParachainServiceSpec};
