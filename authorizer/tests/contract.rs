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
//! are the ones the collator is stuck with, and they are what is under test.

use jam_types::{
	AuthConfig as RawAuthConfig, Authorization, Authorizer, CodeHash, HeaderHash, RefineContext,
	ServiceId, WorkItem, WorkPackage, WorkPayload,
};
use parachain_authorizer::aura::{
	build_collator_tree, signable_work_package_hash, AuthConfig, AuthToken, CollatorKey,
	CollatorSignature, Command, SignatureScheme,
};
use parachain_authorizer_ed25519::Ed25519;
use parachain_authorizer_sr25519::Sr25519;
use parachain_service_interface::types::ParaId;
use sp_core::crypto::KeyTypeId;
use sp_keystore::{testing::MemoryKeystore, Keystore};

/// The aura key type, which is the collator identity phase 6a settles on.
const AURA: KeyTypeId = KeyTypeId(*b"aura");
const PARACHAIN_SERVICE: ServiceId = 5;

#[derive(Clone, Copy, Debug)]
enum Scheme {
	Ed25519,
	Sr25519,
}

/// A collator set as the node holds it: keys in a keystore, signed for through the same
/// `Keystore` calls the collator makes.
struct Collators {
	keystore: MemoryKeystore,
	scheme: Scheme,
	keys: Vec<CollatorKey>,
}

impl Collators {
	fn new(scheme: Scheme, seeds: &[&str]) -> Self {
		let keystore = MemoryKeystore::new();
		let keys = seeds
			.iter()
			.map(|seed| match scheme {
				Scheme::Ed25519 => keystore
					.ed25519_generate_new(AURA, Some(seed))
					.expect("the in-memory keystore generates; qed")
					.0,
				Scheme::Sr25519 => keystore
					.sr25519_generate_new(AURA, Some(seed))
					.expect("the in-memory keystore generates; qed")
					.0,
			})
			.collect();
		Self { keystore, scheme, keys }
	}

	fn sign(&self, index: usize, payload: &[u8]) -> CollatorSignature {
		let key = self.keys[index];
		match self.scheme {
			Scheme::Ed25519 => self
				.keystore
				.ed25519_sign(AURA, &key.into(), payload)
				.expect("signing does not fail; qed")
				.expect("the key is in the keystore; qed")
				.0,
			Scheme::Sr25519 => self
				.keystore
				.sr25519_sign(AURA, &key.into(), payload)
				.expect("signing does not fail; qed")
				.expect("the key is in the keystore; qed")
				.0,
		}
	}

	/// The config a core running `para` under this set is assigned, and one token per collator
	/// signing `package` with `command`.
	fn tokens(
		&self,
		package: &WorkPackage,
		command: Option<Command>,
	) -> (AuthConfig, Vec<AuthToken>) {
		let (collator_set_root, proofs) = build_collator_tree(&self.keys);
		let config = AuthConfig {
			para_ids: vec![ParaId(0)],
			parachain_service: PARACHAIN_SERVICE,
			collator_set_root,
			collator_set_size: self.keys.len() as u32,
			slot_duration: 1,
		};
		let payload =
			AuthToken::signing_payload(signable_work_package_hash(package), &command);
		let tokens = (0..self.keys.len())
			.map(|index| AuthToken {
				proof: proofs[index].clone(),
				key: self.keys[index],
				signature: self.sign(index, payload.as_bytes()),
				control_command: command.clone(),
			})
			.collect();
		(config, tokens)
	}
}

/// A package shaped like the ones the collator submits: one item for the parachain service, and
/// an anchor that is part of what gets signed.
fn package(anchor_byte: u8) -> WorkPackage {
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
			lookup_anchor_slot: 0,
			prerequisites: Default::default(),
		},
		items: vec![WorkItem {
			service: PARACHAIN_SERVICE,
			code_hash: CodeHash([0xbb; 32]),
			refine_gas_limit: 1_000_000,
			accumulate_gas_limit: 1_000_000,
			export_count: 0,
			payload: WorkPayload(b"contract".to_vec()),
			import_segments: Default::default(),
			extrinsics: Default::default(),
		}]
		.try_into()
		.expect("one item is within the JAM bound; qed"),
	}
}

/// Run `check_proof` + `check_signature` the way the guest's `is_authorized` does.
fn verify<S: SignatureScheme>(
	token: &AuthToken,
	config: &AuthConfig,
	index: u32,
	package: &WorkPackage,
) -> bool {
	token.check_proof(config, index).is_ok() &&
		token.check_signature::<S>(signable_work_package_hash(package)).is_ok()
}

/// The whole point: what the node builds, the guest of the same scheme accepts — at every index
/// of the round-robin, so the proof and the signature are both exercised off leaf zero.
#[test]
fn a_keystore_token_passes_its_own_verifier_works() {
	let package = package(1);

	let ed = Collators::new(Scheme::Ed25519, &["//Alice", "//Bob", "//Charlie"]);
	let (config, tokens) = ed.tokens(&package, None);
	for (index, token) in tokens.iter().enumerate() {
		assert!(
			verify::<Ed25519>(token, &config, index as u32, &package),
			"ed25519 collator {index} was rejected by the ed25519 verifier"
		);
	}

	let sr = Collators::new(Scheme::Sr25519, &["//Alice", "//Bob", "//Charlie"]);
	let (config, tokens) = sr.tokens(&package, None);
	for (index, token) in tokens.iter().enumerate() {
		assert!(
			verify::<Sr25519>(token, &config, index as u32, &package),
			"sr25519 collator {index} was rejected by the sr25519 verifier"
		);
	}
}

/// A core's queue commits to one verifier blob, so pointing it at the wrong one must reject
/// everything rather than accept anything. This is also the only automated guard on sr25519's
/// transcript context: a context other than the keystore's fails here indistinguishably from a
/// wrong curve, which is why the positive test above has to pass at the same time.
#[test]
fn a_token_is_rejected_by_the_other_verifier_works() {
	let package = package(1);

	let ed = Collators::new(Scheme::Ed25519, &["//Alice"]);
	let (config, tokens) = ed.tokens(&package, None);
	assert!(!verify::<Sr25519>(&tokens[0], &config, 0, &package));

	let sr = Collators::new(Scheme::Sr25519, &["//Alice"]);
	let (config, tokens) = sr.tokens(&package, None);
	assert!(!verify::<Ed25519>(&tokens[0], &config, 0, &package));
}

/// The signature is over the package's context, so a token cannot be lifted onto a package built
/// against a different anchor — which is what replaying somebody else's authorization would take.
#[test]
fn a_token_does_not_carry_over_to_a_re_anchored_package_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let signed = package(1);
		let reanchored = package(2);
		let collators = Collators::new(scheme, &["//Alice"]);
		let (config, tokens) = collators.tokens(&signed, None);

		let (accepted, rejected) = match scheme {
			Scheme::Ed25519 => (
				verify::<Ed25519>(&tokens[0], &config, 0, &signed),
				verify::<Ed25519>(&tokens[0], &config, 0, &reanchored),
			),
			Scheme::Sr25519 => (
				verify::<Sr25519>(&tokens[0], &config, 0, &signed),
				verify::<Sr25519>(&tokens[0], &config, 0, &reanchored),
			),
		};
		assert!(accepted, "{scheme:?}: the token did not pass on its own package");
		assert!(!rejected, "{scheme:?}: the token passed on a re-anchored package");
	}
}

/// The control command travels in the token, and the package hash deliberately excludes the
/// token — so unless the signature covers the command, anyone can bolt one onto a package they
/// intercept in flight and reassign a core with somebody else's signature.
#[test]
fn a_signature_does_not_carry_over_to_another_command_works() {
	let command = Command::Assign { para_id: ParaId(3), core: 1, authorizer: [0xcd; 32] };
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let package = package(1);
		let collators = Collators::new(scheme, &["//Alice"]);
		let (config, plain) = collators.tokens(&package, None);
		let (_, commanding) = collators.tokens(&package, Some(command.clone()));

		// The token that carries the command, signed for no command at all.
		let forged = AuthToken { control_command: Some(command.clone()), ..plain[0].clone() };

		let verified: Vec<bool> = [&plain[0], &commanding[0], &forged]
			.iter()
			.map(|token| match scheme {
				Scheme::Ed25519 => verify::<Ed25519>(token, &config, 0, &package),
				Scheme::Sr25519 => verify::<Sr25519>(token, &config, 0, &package),
			})
			.collect();
		assert_eq!(verified, vec![true, true, false], "{scheme:?}");
	}
}
