//! Cumulus authorship interface for Collators.
//!
//! All needed exports should be contained in this module. This is done to maintain a clean
//! interface between internal and external dependant code. Please open an issue if anything is
//! missing instead of accessing the other crates directly. You might still need to use the
//! `jam-types` crate, but please check that it is the same version as used here.

pub use ::{
	jam_types::WorkPackage,
	parachain_service::{refine::ParachainCandidate, work_digest::ValidationCodeHash},
	parachain_service_interface::types::ParaId,
	primitive_types::H256,
};

pub mod aura {
	pub use parachain_authorizer::aura::{
		build_collator_tree, expected_collator_index, signable_work_package_hash, AuthConfig,
		AuthToken, AuthTrace, CollatorKey, CollatorSignature, SignatureScheme, TokenError,
		WORK_PACKAGE_SIGN_CTX,
	};

	/// The two verifier blobs' signature schemes, so a collator can check a token the way the
	/// guest will before it spends a core on it. Which one a para uses is its runtime's
	/// `AuraId`, and it decides which blob's code hash the core's queue must hold.
	pub use ::{parachain_authorizer_ed25519::Ed25519, parachain_authorizer_sr25519::Sr25519};
}
