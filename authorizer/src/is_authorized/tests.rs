//! Keystore signing and guest verification for both schemes, including wrong-scheme rejection.
//! sr25519 signatures must use the keystore's `substrate` transcript context.

use crate::aura::{
	build_collator_proof, signable_work_package_hash, AuthConfig, AuthToken, AuthorizationError,
	CollatorKey, CollatorSignature, ConfigError, TokenError,
};
use codec::{DecodeAll as _, Encode};
use jam_types::{
	AuthConfig as RawAuthConfig, Authorization, Authorizer, CodeHash, HeaderHash, RefineContext,
	Slot, WorkItem, WorkPackage, WorkPayload,
};
use parachain_authorizer_ed25519::Ed25519;
use parachain_authorizer_sr25519::Sr25519;
use parachain_service_core::{
	authorization::AuthTrace,
	types::ParaId,
	upward_message::{UpwardMessage, UpwardMessages},
	PARACHAIN_SERVICE_ID,
};
use primitive_types::H256;

/// Independent token layout from design §7.1, to check wire compatibility.
#[derive(Encode)]
struct SpecToken {
	proof: Vec<H256>,
	key: CollatorKey,
	signature: CollatorSignature,
}

/// The aura key type, which is the collator identity phase 6a settles on.
const AURA: sp_core::crypto::KeyTypeId = sp_core::crypto::KeyTypeId(*b"aura");

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

	/// The config a core running `para_ids` under this set is assigned. An empty list is a
	/// *parked* core: same collator set, same authorizer code, no para.
	fn config(&self, para_ids: Vec<ParaId>) -> AuthConfig {
		let (collator_set_root, _) =
			build_collator_proof(&self.keys, 0).expect("valid test collator set");
		AuthConfig {
			para_ids: para_ids.into_iter().map(|id| u32::from(id).into()).collect(),
			parachain_service: PARACHAIN_SERVICE_ID,
			collator_set_root,
			collator_set_size: core::num::NonZero::new(self.keys.len() as u32)
				.expect("nonzero configuration"),
			slot_duration: core::num::NonZero::new(1).expect("nonzero configuration"),
		}
	}

	/// One token per collator, each signing `package` as that collator would.
	fn tokens(&self, package: &WorkPackage) -> Vec<AuthToken> {
		let payload = signable_work_package_hash(package);
		(0..self.keys.len())
			.map(|index| AuthToken {
				proof: build_collator_proof(&self.keys, index as u32)
					.expect("valid test collator set")
					.1,
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
		auth_code_host: PARACHAIN_SERVICE_ID,
		authorizer: Authorizer {
			code_hash: CodeHash([0xaa; 32]),
			config: RawAuthConfig::default(),
		},
		context: RefineContext {
			anchor: HeaderHash([anchor_byte; 32]),
			anchor_slot: slot,
			state_root: Default::default(),
			beefy_root: Default::default(),
			lookup_anchor: HeaderHash([anchor_byte; 32]),
			lookup_anchor_slot: slot,
			lookup_anchor_state_root: Default::default(),
			prerequisites: Default::default(),
		},
		items: vec![WorkItem {
			service: PARACHAIN_SERVICE_ID,
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

/// The package a core assignment rides in: the upward messages *are* the payload.
fn controlling(anchor_byte: u8, slot: Slot) -> WorkPackage {
	let messages: UpwardMessages = vec![UpwardMessage::AssignCore {
		core: 1,
		queue: vec![[0xcd; 32]],
		new_assigner: None,
		jam_slot: 0,
	}]
	.try_into()
	.expect("one message is within the digest bound; qed");
	package(anchor_byte, slot, messages.encode())
}

/// Run the decision the guest runs, under `scheme`'s verifier.
fn authorize_under(
	scheme: Scheme,
	config: &AuthConfig,
	token: &AuthToken,
	package: &WorkPackage,
) -> Result<AuthTrace, AuthorizationError> {
	let slot = package.context.anchor_slot;
	match scheme {
		Scheme::Ed25519 => AuthTrace::try_from_token::<Ed25519>(config, token, package, slot),
		Scheme::Sr25519 => AuthTrace::try_from_token::<Sr25519>(config, token, package, slot),
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
		let config = collators.config(vec![ParaId::new(0)]);
		// The round-robin picks the collator from the anchor slot, so walking the slot is
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

/// A token in the spec's three-field shape is admitted by the guest of its own scheme.
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
		let (_, proof) = build_collator_proof(&collators.keys, 1).expect("valid test collator set");
		let payload = signable_work_package_hash(&package);
		let spec = SpecToken {
			proof,
			key: collators.keys[1],
			signature: collators.sign(1, payload.as_bytes()),
		};

		let token = AuthToken::decode_all(&mut &spec.encode()[..])
			.expect("the guest decodes the spec's three fields and nothing more");
		assert!(
			admits(scheme, &collators.config(vec![ParaId::new(0)]), &token, &package),
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
		&ed.config(vec![ParaId::new(0)]),
		&ed.tokens(&package)[0],
		&package
	));

	let sr = Collators::new(Scheme::Sr25519, &["//Alice"]);
	assert!(!admits(
		Scheme::Ed25519,
		&sr.config(vec![ParaId::new(0)]),
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
		let config = collators.config(vec![ParaId::new(0)]);
		let token = &collators.tokens(&signed)[0];

		assert!(admits(scheme, &config, token, &signed), "{scheme:?}: rejected on its own package");
		assert!(
			!admits(scheme, &config, token, &reanchored),
			"{scheme:?}: passed on a re-anchored package"
		);
	}
}

/// The local JAM adapter must sign context fields absent from published jam-types 0.1.28.
#[test]
fn newer_context_fields_errors() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let signed = block(1, 0);
		let collators = Collators::new(scheme, &["//Alice"]);
		let config = collators.config(vec![ParaId::new(0)]);
		let token = &collators.tokens(&signed)[0];
		assert!(admits(scheme, &config, token, &signed));

		let mut changed_slot = signed.clone();
		changed_slot.context.anchor_slot += 1;
		assert!(!admits(scheme, &config, token, &changed_slot));

		let mut changed_root = signed.clone();
		changed_root.context.lookup_anchor_state_root.0[0] ^= 1;
		assert!(!admits(scheme, &config, token, &changed_root));
	}
}

/// A core assignment now travels in a work item, so the package hash covers it: a signature
/// cannot be lifted from an innocent package onto one that reassigns a core. Phase 6 bound the
/// command into the signing payload by hand because it lived outside the hash; this is that
/// property, held by the hash itself.
#[test]
fn a_signature_does_not_carry_over_to_another_payload_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let signed = block(1, 0);
		let collators = Collators::new(scheme, &["//Alice"]);
		let config = collators.config(vec![ParaId::new(0)]);
		let token = &collators.tokens(&signed)[0];

		assert!(admits(scheme, &config, token, &signed), "{scheme:?}: rejected on its own package");
		assert!(
			!admits(scheme, &config, token, &controlling(1, 0)),
			"{scheme:?}: an innocent package's signature authorized a core assignment"
		);
	}
}

/// A parked core has the same authorizer code, but no para. Every ordinary
/// package must bounce off it — the item-count check has nothing to match — which is what stops a
/// collator spending a core that no longer runs its para.
#[test]
fn a_parked_core_admits_no_ordinary_package_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice"]);
		let parked = collators.config(Vec::new());
		let package = block(1, 0);
		// A perfectly good collator signature is still not a para assignment.
		let token = &collators.tokens(&package)[0];
		assert!(matches!(
			authorize_under(scheme, &parked, token, &package),
			Err(AuthorizationError::BadConfig(ConfigError::InvalidWorkItemCount))
		));
		// Nor does putting an assignment in it help without the privilege to run one.
		let control = controlling(1, 0);
		assert!(matches!(
			authorize_under(scheme, &parked, &collators.tokens(&control)[0], &control),
			Err(AuthorizationError::BadConfig(ConfigError::InvalidWorkItemCount))
		));
	}
}

/// The former development sentinel must pass the same checks as every other key.
#[test]
fn unsigned_sentinel_errors() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice"]);
		let package = block(1, 0);
		let forged = AuthToken { proof: Vec::new(), key: [0xff; 32], signature: [0; 64] };
		assert!(matches!(
			authorize_under(scheme, &collators.config(Vec::new()), &forged, &package),
			Err(AuthorizationError::BadConfig(ConfigError::InvalidWorkItemCount))
		));
		assert!(matches!(
			authorize_under(scheme, &collators.config(vec![ParaId::new(0)]), &forged, &package),
			Err(AuthorizationError::BadToken(_))
		));
		// Even membership cannot turn the former sentinel into an unsigned authorization.
		let mut config = collators.config(vec![ParaId::new(0)]);
		config.collator_set_root = build_collator_proof(&[forged.key], 0).unwrap().0;
		assert!(matches!(
			authorize_under(scheme, &config, &forged, &package),
			Err(AuthorizationError::BadToken(TokenError::BadCollatorSignature))
		));
		let mut foreign = package;
		foreign.items[0].service = PARACHAIN_SERVICE_ID + 1;
		assert!(matches!(
			authorize_under(scheme, &collators.config(vec![ParaId::new(0)]), &forged, &foreign),
			Err(AuthorizationError::BadConfig(ConfigError::InvalidWorkItemService))
		));
	}
}

/// Assigned cores are unaffected by any of the above: their behaviour is exactly what it was.
#[test]
fn an_assigned_core_still_counts_items_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice"]);
		let package = block(1, 0);
		let token = &collators.tokens(&package)[0];
		// Two paras assigned, one item submitted.
		assert!(matches!(
			authorize_under(
				scheme,
				&collators.config(vec![ParaId::new(0), ParaId::new(1)]),
				token,
				&package
			),
			Err(AuthorizationError::BadConfig(ConfigError::InvalidWorkItemCount))
		));
	}
}
