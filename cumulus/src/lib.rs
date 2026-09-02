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
	// TODO(contract-1): `signable_work_package_hash` (jam-codec, ctx
	// `jam:parachain-service:aura:work-package:v1`) is the canonical signed work-package hash for
	// phase 1. It diverges from polkajam's `signed-authorizer` `create_message`; before phase 3
	// one encoding must be picked, the others aliased, and a cross-repo test added.
	pub use parachain_authorizer::aura::{
		build_collator_tree, expected_collator_index, signable_work_package_hash, AuthConfig,
		AuthToken, AuthTrace, CollatorKey, CollatorSignature, SignatureScheme, TokenError,
		WORK_PACKAGE_SIGN_CTX,
	};

	/// The core-assignment command, and the work-item payload prefix that carries it. Not the
	/// authorizer's: since 6a.4 a command travels in the package rather than in the token, so it
	/// belongs to the service the item is addressed to.
	pub use parachain_service_interface::authorization::{Command, CONTROL_COMMAND_PREFIX};

	/// The two verifier blobs' signature schemes, so a collator can check a token the way the
	/// guest will before it spends a core on it. Which one a para uses is its runtime's `AuraId`,
	/// and it decides which blob's code hash the core's queue must hold.
	pub use ::{parachain_authorizer_ed25519::Ed25519, parachain_authorizer_sr25519::Sr25519};
}

pub mod service_state {
	//! The parachain service's per-para state entry, as read by the collator over
	//! `serviceValue`: key `[0x00] ‖ SCALE(ParaId)` → SCALE-encoded [`ParaInfo`]
	//! (spec §3.1; `head_data` is the para head the collator follows).

	pub use parachain_service::state::{para_info::ParaInfo, storage_key, Tag};
	use parachain_service_interface::types::ParaId;

	/// Storage key of the para's [`ParaInfo`] entry in the parachain service.
	pub fn para_info_key(para_id: ParaId) -> Vec<u8> {
		storage_key(Tag::Parachains, &para_id)
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn para_info_key_is_tag_then_scale_para_id() {
			assert_eq!(para_info_key(ParaId::from(3)), vec![0x00, 3, 0, 0, 0]);
		}
	}
}

pub mod authorizer {
	//! The authorizer-hash contract (contract 2).
	//!
	//! The Coretime chain (writes the queue), the collator (scans and signs) and the PVF (decodes
	//! the config) must hash byte-identical blobs. The layout, pinned by
	//! `jam_types::Authorizer::{with_concat, hash}`, is
	//! `blake2b-256(code_hash ‖ config)` — a raw concatenation of the 32 code-hash bytes and the
	//! raw config blob. No domain separator, no SCALE struct wrapper.

	pub use jam_types::{AuthConfig as AuthConfigBlob, Authorizer, AuthorizerHash, CodeHash};

	/// Compute the authorizer hash: `blake2b-256(code_hash ‖ config)`.
	pub fn authorizer_hash(authorizer: &Authorizer) -> AuthorizerHash {
		authorizer.hash(blake2b_256)
	}

	fn blake2b_256(data: &[u8]) -> jam_types::Hash {
		let mut hash = [0u8; 32];
		hash.copy_from_slice(
			blake2b_simd::Params::new().hash_length(32).hash(data).as_bytes(),
		);
		hash
	}

	/// Code hash of polkajam's null authorizer (accepts anything). Dev/testnet genesis fills
	/// every core's queue with this authorizer.
	pub const NULL_AUTHORIZER_CODE_HASH: [u8; 32] = jam_null_authorizer_bin::HASH;

	/// The hardcoded phase-1 authorizer: the null authorizer with an empty config.
	pub fn fixed_authorizer() -> Authorizer {
		Authorizer {
			code_hash: NULL_AUTHORIZER_CODE_HASH.into(),
			config: AuthConfigBlob(Vec::new()),
		}
	}

	/// `authorizer_hash(fixed_authorizer())`, precomputed. This is the hash the phase-1 core
	/// scan looks for in the authorizer queues.
	pub const FIXED_AUTHORIZER_HASH: AuthorizerHash = AuthorizerHash([
		35, 87, 66, 111, 35, 19, 85, 154, 39, 29, 103, 130, 220, 0, 25, 123, 55, 159, 121, 203,
		227, 198, 161, 231, 47, 97, 247, 181, 146, 197, 9, 248,
	]);


	#[cfg(test)]
	mod tests {
		use super::*;

		#[test]
		fn fixed_authorizer_hash_matches_helper() {
			assert_eq!(authorizer_hash(&fixed_authorizer()), FIXED_AUTHORIZER_HASH);
		}

		#[test]
		fn authorizer_hash_is_raw_concat_not_scale() {
			let authorizer = Authorizer {
				code_hash: [7u8; 32].into(),
				config: AuthConfigBlob(vec![1, 2, 3]),
			};
			let concat: Vec<u8> = [&[7u8; 32][..], &[1, 2, 3][..]].concat();
			let expected = AuthorizerHash(blake2b_256(&concat));
			assert_eq!(authorizer_hash(&authorizer), expected);
		}
	}
}
