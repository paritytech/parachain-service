//! Cumulus authorship interface for Collators.
//!
//! All needed exports should be contained in this module. This is done to maintain a clean
//! interface between internal and external dependant code. Please open an issue if anything is
//! missing instead of accessing the other crates directly. You might still need to use the
//! `jam-types` crate, but please check that it is the same version as used here.

pub use ::{
	jam_types::WorkPackage,
	parachain_service::{refine::CandidatePayload, work_digest::ValidationCodeHash},
	parachain_support::types::ParaId,
	primitive_types::H256,
};

pub mod aura {
	pub use parachain_authorizer::aura::{
		signable_work_package_hash, AuthConfig, AuthToken, CollatorKey, CollatorSignature,
		TokenError, WORK_PACKAGE_SIGN_CTX,
	};
}
