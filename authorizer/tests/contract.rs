//! The contract between the node that assembles an authorization token and the guest that
//! verifies it, for both schemes and across them.
//!
//! Both schemes live in one file because the interesting failures are the cross-scheme ones: a
//! token checked under the wrong curve fails exactly the way a forged one does, with nothing in
//! the error to say which it was. sr25519 is only correct at all if this repo and
//! `sp_core::sr25519`'s hard-coded `b"substrate"` transcript context agree — nothing else would
//! notice them drifting apart, and the symptom on a live network is every package silently
//! failing to authorize.
//!
//! So the signing here goes through `Keystore`, never through a `Pair`: the keystore's choices
//! are the ones the collator is stuck with, and they are what is under test. And the decision
//! under test is `is_authorized::authorize` itself, not a re-implementation of it — a carve-out
//! that only the tests know about would guard nothing.

use codec::{DecodeAll as _, Encode};
use jam_types::{
	AuthConfig as RawAuthConfig, Authorization, Authorizer, CodeHash, HeaderHash, RefineContext,
	ServiceId, Slot, WorkItem, WorkPackage, WorkPayload,
};
use parachain_authorizer::{
	aura::{
		build_collator_tree, signable_work_package_hash, AuthConfig, AuthToken, AuthTrace,
		CollatorKey, CollatorSignature,
	},
	is_authorized::{authorize, AuthorizationError},
};
use parachain_authorizer_ed25519::Ed25519;
use parachain_authorizer_sr25519::Sr25519;
use parachain_service_interface::types::ParaId;
use primitive_types::H256;

/// The token exactly as the design spec writes it (§7.1: `AuthToken { proof, key, signature }`),
/// declared here rather than reused so that the test encodes what the *spec* says while the guest
/// decodes what the *crate* says.
#[derive(Encode)]
struct SpecToken {
	proof: Vec<H256>,
	key: CollatorKey,
	signature: CollatorSignature,
}

/// The aura key type, which is the collator identity phase 6a settles on.
const AURA: sp_core::crypto::KeyTypeId = sp_core::crypto::KeyTypeId(*b"aura");
const PARACHAIN_SERVICE: ServiceId = 5;

#[derive(Clone, Copy, Debug)]
enum Scheme {
	Ed25519,
	Sr25519,
}

/// A collator set as the node holds it: keys in a keystore, signed for through the same
/// `Keystore` calls the collator makes.
struct Collators {
	keystore: sp_keystore::testing::MemoryKeystore,
	scheme: Scheme,
	keys: Vec<CollatorKey>,
}

impl Collators {
	fn new(scheme: Scheme, seeds: &[&str]) -> Self {
		use sp_keystore::Keystore as _;
		let keystore = sp_keystore::testing::MemoryKeystore::new();
		let keys = seeds
			.iter()
			.map(|seed| match scheme {
				Scheme::Ed25519 => {
					keystore
						.ed25519_generate_new(AURA, Some(seed))
						.expect("the in-memory keystore generates; qed")
						.0
				},
				Scheme::Sr25519 => {
					keystore
						.sr25519_generate_new(AURA, Some(seed))
						.expect("the in-memory keystore generates; qed")
						.0
				},
			})
			.collect();
		Self { keystore, scheme, keys }
	}

	fn sign(&self, index: usize, payload: &[u8]) -> CollatorSignature {
		use sp_keystore::Keystore as _;
		let key = self.keys[index];
		match self.scheme {
			Scheme::Ed25519 => {
				self.keystore
					.ed25519_sign(AURA, &key.into(), payload)
					.expect("signing does not fail; qed")
					.expect("the key is in the keystore; qed")
					.0
			},
			Scheme::Sr25519 => {
				self.keystore
					.sr25519_sign(AURA, &key.into(), payload)
					.expect("signing does not fail; qed")
					.expect("the key is in the keystore; qed")
					.0
			},
		}
	}

	/// The config a core running `para_ids` under this set is assigned at genesis.
	fn config(&self, para_ids: Vec<ParaId>) -> AuthConfig {
		let (collator_set_root, _) = build_collator_tree(&self.keys);
		AuthConfig {
			para_ids,
			parachain_service: PARACHAIN_SERVICE,
			collator_set_root,
			collator_set_size: self.keys.len() as u32,
			slot_duration: 1,
		}
	}

	/// One token per collator, each signing `package` as that collator would.
	fn tokens(&self, package: &WorkPackage) -> Vec<AuthToken> {
		let (_, proofs) = build_collator_tree(&self.keys);
		let payload = signable_work_package_hash(package);
		(0..self.keys.len())
			.map(|index| AuthToken {
				proof: proofs[index].clone(),
				key: self.keys[index],
				signature: self.sign(index, payload.as_bytes()),
			})
			.collect()
	}
}

/// A package shaped like the ones the collator submits: one item for the parachain service, and
/// an anchor and a slot that are both part of what gets signed.
fn package(anchor_byte: u8, slot: Slot, payload: Vec<u8>) -> WorkPackage {
	WorkPackage {
		authorization: Authorization::default(),
		auth_code_host: PARACHAIN_SERVICE,
		authorizer: Authorizer {
			code_hash: CodeHash([0xaa; 32]),
			config: RawAuthConfig::default(),
		},
		context: RefineContext {
			anchor: HeaderHash([anchor_byte; 32]),
			state_root: Default::default(),
			beefy_root: Default::default(),
			lookup_anchor: HeaderHash([anchor_byte; 32]),
			lookup_anchor_slot: slot,
			prerequisites: Default::default(),
		},
		items: vec![WorkItem {
			service: PARACHAIN_SERVICE,
			code_hash: CodeHash([0xbb; 32]),
			refine_gas_limit: 1_000_000,
			accumulate_gas_limit: 1_000_000,
			export_count: 0,
			payload: WorkPayload(payload),
			import_segments: Default::default(),
			extrinsics: Default::default(),
		}]
		.try_into()
		.expect("one item is within the JAM bound; qed"),
	}
}

/// A parachain block, as far as the authorizer is concerned: any payload at all.
fn block(anchor_byte: u8, slot: Slot) -> WorkPackage {
	package(anchor_byte, slot, b"contract".to_vec())
}

/// Run the decision the guest runs, under `scheme`'s verifier.
fn authorize_under(
	scheme: Scheme,
	config: &AuthConfig,
	token: &AuthToken,
	package: &WorkPackage,
) -> Result<AuthTrace, AuthorizationError> {
	let slot = package.context.lookup_anchor_slot;
	match scheme {
		Scheme::Ed25519 => authorize::<Ed25519>(config, token, package, slot),
		Scheme::Sr25519 => authorize::<Sr25519>(config, token, package, slot),
	}
}

fn admits(scheme: Scheme, config: &AuthConfig, token: &AuthToken, package: &WorkPackage) -> bool {
	authorize_under(scheme, config, token, package).is_ok()
}

/// The whole point: what the node builds, the guest of the same scheme accepts — at every index
/// of the round-robin, so the proof and the signature are both exercised off leaf zero.
#[test]
fn a_keystore_token_passes_its_own_verifier_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice", "//Bob", "//Charlie"]);
		let config = collators.config(vec![ParaId(0)]);
		// The round-robin picks the collator from the lookup-anchor slot, so walking the slot is
		// what walks the set.
		for index in 0..collators.keys.len() {
			let package = block(1, index as Slot);
			let token = &collators.tokens(&package)[index];
			assert!(
				admits(scheme, &config, token, &package),
				"{scheme:?} collator {index} was rejected by its own verifier"
			);
		}
	}
}

/// The compatibility pin: a token in the spec's three-field shape is admitted by the guest of its
/// own scheme.
///
/// `decode_all` is what makes this a wire test rather than a type test — it fails on a trailing
/// byte, so a fourth field appearing in `AuthToken` breaks this even if every other test still
/// compiles.
#[test]
fn a_spec_shaped_token_passes_its_own_verifier_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		// Two collators and slot 1, so the proof is a real sibling rather than an empty vec.
		let collators = Collators::new(scheme, &["//Alice", "//Bob"]);
		let package = block(1, 1);
		let (_, proofs) = build_collator_tree(&collators.keys);
		let payload = signable_work_package_hash(&package);
		let spec = SpecToken {
			proof: proofs[1].clone(),
			key: collators.keys[1],
			signature: collators.sign(1, payload.as_bytes()),
		};

		let token = AuthToken::decode_all(&mut &spec.encode()[..])
			.expect("the guest decodes the spec's three fields and nothing more");
		assert!(
			admits(scheme, &collators.config(vec![ParaId(0)]), &token, &package),
			"{scheme:?}: a spec-shaped token was rejected by its own verifier"
		);
	}
}

/// A core's queue commits to one verifier blob, so pointing it at the wrong one must reject
/// everything rather than accept anything. This is also the only automated guard on sr25519's
/// transcript context: a context other than the keystore's fails here indistinguishably from a
/// wrong curve, which is why the positive test above has to pass at the same time.
#[test]
fn a_token_is_rejected_by_the_other_verifier_works() {
	let package = block(1, 0);

	let ed = Collators::new(Scheme::Ed25519, &["//Alice"]);
	assert!(!admits(
		Scheme::Sr25519,
		&ed.config(vec![ParaId(0)]),
		&ed.tokens(&package)[0],
		&package
	));

	let sr = Collators::new(Scheme::Sr25519, &["//Alice"]);
	assert!(!admits(
		Scheme::Ed25519,
		&sr.config(vec![ParaId(0)]),
		&sr.tokens(&package)[0],
		&package
	));
}

/// The signature is over the package's context, so a token cannot be lifted onto a package built
/// against a different anchor — which is what replaying somebody else's authorization would take.
#[test]
fn a_token_does_not_carry_over_to_a_re_anchored_package_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let signed = block(1, 0);
		let reanchored = block(2, 0);
		let collators = Collators::new(scheme, &["//Alice"]);
		let config = collators.config(vec![ParaId(0)]);
		let token = &collators.tokens(&signed)[0];

		assert!(admits(scheme, &config, token, &signed), "{scheme:?}: rejected on its own package");
		assert!(
			!admits(scheme, &config, token, &reanchored),
			"{scheme:?}: passed on a re-anchored package"
		);
	}
}

/// The work items are inside the signed hash, so a signature cannot be lifted off the block its
/// collator authored and onto somebody else's — which is what submitting a foreign block under a
/// scheduled collator's authorization would take.
#[test]
fn a_signature_does_not_carry_over_to_another_payload_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let signed = block(1, 0);
		let substituted = package(1, 0, b"somebody else's block".to_vec());
		let collators = Collators::new(scheme, &["//Alice"]);
		let config = collators.config(vec![ParaId(0)]);
		let token = &collators.tokens(&signed)[0];

		assert!(admits(scheme, &config, token, &signed), "{scheme:?}: rejected on its own package");
		assert!(
			!admits(scheme, &config, token, &substituted),
			"{scheme:?}: a signature authorized a payload it never covered"
		);
	}
}

/// Cores are assigned at genesis and each config names its paras, one per work item. A package
/// carrying a different number of items is not the package that core's coretime was bought for.
#[test]
fn a_mismatched_item_count_errors() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice"]);
		let package = block(1, 0);
		let token = &collators.tokens(&package)[0];
		// Two paras assigned, one item submitted.
		assert!(matches!(
			authorize_under(scheme, &collators.config(vec![ParaId(0), ParaId(1)]), token, &package),
			Err(AuthorizationError::InvalidWorkItemCount)
		));
	}
}

/// Para-specific coretime must not be spent on other JAM work, however well signed the package
/// is: a collator with a perfectly good token cannot redirect its core at another service.
#[test]
fn a_foreign_target_service_errors() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice"]);
		let mut foreign = block(1, 0);
		foreign.items[0].service = PARACHAIN_SERVICE + 1;
		let token = &collators.tokens(&foreign)[0];

		assert!(matches!(
			authorize_under(scheme, &collators.config(vec![ParaId(0)]), token, &foreign),
			Err(AuthorizationError::WrongTargetService)
		));
	}
}
