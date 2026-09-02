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
		CollatorKey, CollatorSignature, SUDO_KEY,
	},
	is_authorized::{authorize, AuthorizationError},
};
use parachain_authorizer_ed25519::Ed25519;
use parachain_authorizer_sr25519::Sr25519;
use parachain_service_interface::{
	authorization::{Command, CONTROL_COMMAND_PREFIX},
	types::ParaId,
};
use primitive_types::{H256, U256};

/// The token exactly as the design spec writes it (§7.1: `AuthToken { proof, key, signature }`),
/// declared here rather than reused so that the test encodes what the *spec* says while the guest
/// decodes what the *crate* says.
///
/// This is the shape a collator built against the final design emits, knowing nothing of parasim's
/// sudo lane. Keeping the two in step is the entire reason that lane rides a sentinel key instead
/// of a field of its own — a fourth field would make every collator encode a flag forever.
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
		use sp_keystore::Keystore as _;
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

	/// The config a core running `para_ids` under this set is assigned. An empty list is a
	/// *parked* core: same collator set, same authorizer code, no para.
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

/// A parachain block, as far as the authorizer is concerned: any payload without the command
/// prefix.
fn block(anchor_byte: u8, slot: Slot) -> WorkPackage {
	package(anchor_byte, slot, b"contract".to_vec())
}

/// The package a core-assignment command rides in: the command *is* the payload.
fn commanding(anchor_byte: u8, slot: Slot) -> WorkPackage {
	let command = Command::Free { core: 1, parked_authorizer: [0xcd; 32] };
	let mut payload = CONTROL_COMMAND_PREFIX.to_vec();
	command.encode_to(&mut payload);
	package(anchor_byte, slot, payload)
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

/// The compatibility pin: a token in the spec's three-field shape, assembled by someone who has
/// never heard of the sudo lane, is admitted by the guest of its own scheme.
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

/// The sentinel's safety argument, checked rather than left in prose.
///
/// A public key on either curve is the compressed encoding of a point, and the low 255 bits of
/// that encoding are a coordinate reduced modulo the curves' shared field prime, 2^255 − 19.
/// `SUDO_KEY`'s low 255 bits are 2^255 − 1, which is not reduced — so these bytes are not the
/// encoding of any key a keygen can emit, and nobody can find themselves in a collator set under
/// the one key that opens the sudo lane.
#[test]
fn the_sudo_sentinel_is_no_keys_encoding_works() {
	let field_prime = (U256::MAX >> 1) - 18;
	let mut coordinate = SUDO_KEY;
	coordinate[31] &= 0x7f; // The top bit carries a sign, not part of the coordinate.
	assert!(
		U256::from_little_endian(&coordinate) >= field_prime,
		"the sentinel is a reduced field element, so it could be somebody's real key"
	);
}

/// A core's queue commits to one verifier blob, so pointing it at the wrong one must reject
/// everything rather than accept anything. This is also the only automated guard on sr25519's
/// transcript context: a context other than the keystore's fails here indistinguishably from a
/// wrong curve, which is why the positive test above has to pass at the same time.
#[test]
fn a_token_is_rejected_by_the_other_verifier_works() {
	let package = block(1, 0);

	let ed = Collators::new(Scheme::Ed25519, &["//Alice"]);
	assert!(!admits(Scheme::Sr25519, &ed.config(vec![ParaId(0)]), &ed.tokens(&package)[0], &package));

	let sr = Collators::new(Scheme::Sr25519, &["//Alice"]);
	assert!(!admits(Scheme::Ed25519, &sr.config(vec![ParaId(0)]), &sr.tokens(&package)[0], &package));
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

/// A command now travels in a work item, so the package hash covers it: a signature cannot be
/// lifted from an innocent package onto one that reassigns a core. Phase 6 bound the command into
/// the signing payload by hand because it lived outside the hash; this is that property, held by
/// the hash itself.
#[test]
fn a_signature_does_not_carry_over_to_another_payload_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let signed = block(1, 0);
		let collators = Collators::new(scheme, &["//Alice"]);
		let config = collators.config(vec![ParaId(0)]);
		let token = &collators.tokens(&signed)[0];

		assert!(admits(scheme, &config, token, &signed), "{scheme:?}: rejected on its own package");
		assert!(
			!admits(scheme, &config, token, &commanding(1, 0)),
			"{scheme:?}: an innocent package's signature authorized a command"
		);
	}
}

/// A parked core is the one-way end of assignment: same authorizer code, no para. Every ordinary
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
			Err(AuthorizationError::InvalidWorkItemCount)
		));
		// Nor does putting a command in it help without the privilege to run one.
		let control = commanding(1, 0);
		assert!(matches!(
			authorize_under(scheme, &parked, &collators.tokens(&control)[0], &control),
			Err(AuthorizationError::InvalidWorkItemCount)
		));
	}
}

/// The sentinel key is what gets a command onto a parked core, and it is a deliberate hole: no
/// proof and no signature are checked at all. Pinned so that widening it any further has to be a
/// conscious edit — and so that the trace says `sudo`, which is the only thing stopping refine
/// running a command that came in on the ordinary lane.
#[test]
fn a_sudo_token_reaches_a_parked_core_unsigned_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice"]);
		let parked = collators.config(Vec::new());
		let control = commanding(1, 0);

		let forged = AuthToken { proof: Vec::new(), key: SUDO_KEY, signature: [0u8; 64] };
		let trace = authorize_under(scheme, &parked, &forged, &control)
			.expect("the sentinel key admits a package nobody signed");
		assert!(trace.sudo, "{scheme:?}: the trace must tell refine the command may run");
		assert_eq!(trace.author_key, SUDO_KEY);

		// The hole is exactly this wide: the target-service check still applies, so a parked
		// core's coretime cannot be spent on some other JAM service.
		let mut foreign = control;
		foreign.items[0].service = PARACHAIN_SERVICE + 1;
		assert!(matches!(
			authorize_under(scheme, &parked, &forged, &foreign),
			Err(AuthorizationError::WrongTargetService)
		));
	}
}

/// The sentinel key is the *whole* of the signal: nothing else in a token can switch the sudo
/// lane on, and nothing else can switch it off. Pinned because the signal used to be a field of
/// its own — a magic value is easy to compare sloppily, and either mistake is silent.
#[test]
fn only_the_sentinel_key_opens_the_sudo_lane_works() {
	for scheme in [Scheme::Ed25519, Scheme::Sr25519] {
		let collators = Collators::new(scheme, &["//Alice"]);
		let control = commanding(1, 0);

		// A sentinel token dressed up as an ordinary one — real proof, real signature — is still
		// the sudo lane, on a parked core and on an assigned one alike: the key decides before
		// anything else in the token is looked at.
		let mut dressed = collators.tokens(&control)[0].clone();
		dressed.key = SUDO_KEY;
		for config in [collators.config(Vec::new()), collators.config(vec![ParaId(0)])] {
			let trace = authorize_under(scheme, &config, &dressed, &control)
				.expect("the sentinel decides, whatever else the token carries");
			assert!(trace.sudo, "{scheme:?}: a dressed-up sentinel lost its privilege");
		}

		// One byte off the sentinel is an ordinary collator key, and an ordinary key on a parked
		// core buys nothing at all.
		let mut near_miss = dressed;
		near_miss.key[31] = 0xfe;
		assert!(matches!(
			authorize_under(scheme, &collators.config(Vec::new()), &near_miss, &control),
			Err(AuthorizationError::InvalidWorkItemCount)
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
			authorize_under(scheme, &collators.config(vec![ParaId(0), ParaId(1)]), token, &package),
			Err(AuthorizationError::InvalidWorkItemCount)
		));
	}
}

