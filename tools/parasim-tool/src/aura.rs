//! The AURA authorizer a package runs under: the collator set, the config it commits to, and the
//! token that satisfies it.
//!
//! The set is named, never supplied: `--collators alice,bob` derives the same `//Name` keys the
//! runtime's genesis puts in `authorities()` and the harness puts in a collator's keystore, on
//! whichever curve `--scheme` names. Everything else — the trie, the leaf hashing, the signing
//! payload — comes out of `parachain-authorizer`, the crate the guest is built from, because a
//! queue hash commits to the whole config and a hash nobody can reproduce is a core nobody can
//! use.

use codec::{DecodeAll as _, Encode as _};
use jam_types::{
	AuthConfig as RawAuthConfig, Authorization, Authorizer, AuthorizerHash, CodeHash, ServiceId,
	WorkPackage,
};
use parachain_authorizer::aura::{
	build_collator_tree, expected_collator_index, signable_work_package_hash, AuthConfig,
	AuthToken, CollatorKey, CollatorSignature, SUDO_KEY,
};
use parachain_service_interface::types::ParaId;
use primitive_types::H256;
use sp_core::{ed25519, sr25519, Pair as _};

/// The curve a para's collators sign on, which is its runtime's `AuraId`.
///
/// It picks the dev keys derived from `--collators` and, with them, the authorizer blob whose
/// code hash the core's queue must hold: there is one verifier blob per scheme and neither can
/// read the other's signatures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Scheme {
	Ed25519,
	Sr25519,
}

impl std::fmt::Display for Scheme {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Ed25519 => "ed25519",
			Self::Sr25519 => "sr25519",
		})
	}
}

/// One collator's dev key.
enum CollatorPair {
	Ed25519(ed25519::Pair),
	Sr25519(sr25519::Pair),
}

impl CollatorPair {
	/// The key behind a dev name, derived exactly as `key insert --suri //Name` would.
	fn from_dev_name(scheme: Scheme, name: &str) -> Result<Self, String> {
		let mut capitalized = name.to_string();
		capitalized[..1].make_ascii_uppercase();
		let suri = format!("//{capitalized}");
		match scheme {
			Scheme::Ed25519 => ed25519::Pair::from_string(&suri, None).map(Self::Ed25519),
			Scheme::Sr25519 => sr25519::Pair::from_string(&suri, None).map(Self::Sr25519),
		}
		.map_err(|e| format!("no {scheme:?} dev key for collator {name:?}: {e:?}"))
	}

	fn public(&self) -> CollatorKey {
		match self {
			Self::Ed25519(pair) => pair.public().0,
			Self::Sr25519(pair) => pair.public().0,
		}
	}

	/// Sign as the collator would, through the same `sp_core` pairs the keystore is made of —
	/// sr25519's transcript context in particular is not ours to pick.
	fn sign(&self, payload: &[u8]) -> CollatorSignature {
		match self {
			Self::Ed25519(pair) => pair.sign(payload).0,
			Self::Sr25519(pair) => pair.sign(payload).0,
		}
	}
}

/// A para's AURA authorizer, and the keys needed to author under it.
pub struct Aura {
	/// Code hash of the deployed authorizer blob. The queue holds `blake2b(code_hash ‖ config)`,
	/// so this and the config together are the core's identity.
	pub code_hash: CodeHash,
	/// The JAM service the authorizer admits work items for, and nothing else.
	pub service: ServiceId,
	/// Length of a para slot in JAM timeslots; the round-robin's divisor.
	pub slot_duration: u32,
	/// The curve `pairs` are on. Not part of the config — it is the blob `code_hash` names — but
	/// it is what a hash this tool recognises has to be labelled with.
	pub scheme: Scheme,
	/// The dev names `pairs` were derived from, in round-robin order. Kept because a label on a
	/// recognised hash has to say which set it was recognised as, or nobody can act on it.
	pub collators: String,
	pairs: Vec<CollatorPair>,
	root: H256,
	proofs: Vec<Vec<H256>>,
}

impl Aura {
	/// Derive the collator set from dev names, in the order the round-robin walks it.
	pub fn from_dev_names(
		names: &str,
		scheme: Scheme,
		code_hash: CodeHash,
		service: ServiceId,
		slot_duration: u32,
	) -> Result<Self, String> {
		let pairs = names
			.split(',')
			.map(|name| CollatorPair::from_dev_name(scheme, name.trim()))
			.collect::<Result<Vec<_>, _>>()?;
		if pairs.is_empty() {
			return Err("--collators names at least one collator".to_string());
		}
		let keys: Vec<CollatorKey> = pairs.iter().map(CollatorPair::public).collect();
		let (root, proofs) = build_collator_tree(&keys);
		let collators = names.split(',').map(str::trim).collect::<Vec<_>>().join(",");
		Ok(Self { code_hash, service, slot_duration, scheme, collators, pairs, root, proofs })
	}

	/// The config for a core dedicated to `para`: one work item, one para.
	pub fn config(&self, para: ParaId) -> AuthConfig {
		AuthConfig {
			para_ids: vec![para],
			parachain_service: self.service,
			collator_set_root: self.root,
			collator_set_size: self.pairs.len() as u32,
			slot_duration: self.slot_duration,
		}
	}

	/// The config of a *parked* core: this collator set, no para.
	///
	/// The same authorizer code as an assigned core, so a parked core still admits the `sudo`
	/// package that would re-assign it; no para, so the item-count check refuses every parachain
	/// block sent to it. That is what makes assignment one-way — a core never goes back to the
	/// null authorizer, which would leave it deaf to commands.
	pub fn parked_config(&self) -> AuthConfig {
		AuthConfig { para_ids: Vec::new(), ..self.config(ParaId(0)) }
	}

	pub fn authorizer(&self, para: ParaId) -> Authorizer {
		self.wrap(self.config(para))
	}

	pub fn parked_authorizer(&self) -> Authorizer {
		self.wrap(self.parked_config())
	}

	/// What a core's authorizer queue holds when it is running `para`.
	pub fn hash(&self, para: ParaId) -> AuthorizerHash {
		self.authorizer(para).hash(jam_std_common::hash_raw)
	}

	/// What a core's authorizer queue holds once it is parked.
	pub fn parked_hash(&self) -> AuthorizerHash {
		self.parked_authorizer().hash(jam_std_common::hash_raw)
	}

	fn wrap(&self, config: AuthConfig) -> Authorizer {
		Authorizer { code_hash: self.code_hash, config: RawAuthConfig(config.encode()) }
	}

	/// Sign `package` as whichever collator its lookup anchor names.
	///
	/// The round-robin index is read out of the config the package itself carries, not out of
	/// this struct's fields, so the token can only ever be built against the config the
	/// authorizer is going to read. Rather than hunt for an anchor that names a particular
	/// collator, this signs as the one the anchor already names — every dev key is to hand.
	pub fn token(&self, package: &WorkPackage) -> Result<Authorization, String> {
		let config = AuthConfig::decode_all(&mut &package.authorizer.config[..])
			.map_err(|e| format!("the package's own authorizer config does not decode: {e}"))?;
		let index = expected_collator_index(package.context.lookup_anchor_slot, &config) as usize;
		let pair = self
			.pairs
			.get(index)
			.ok_or_else(|| format!("the collator set has no member {index}"))?;

		let payload = signable_work_package_hash(package);
		let token = AuthToken {
			proof: self.proofs[index].clone(),
			key: pair.public(),
			signature: pair.sign(payload.as_bytes()),
		};
		tracing::info!(
			"signing as collator {index} of {} (lookup anchor slot {})",
			self.pairs.len(),
			package.context.lookup_anchor_slot
		);
		Ok(Authorization(token.encode()))
	}
}

/// `names` reordered ascending by public key, which is the order a substrate runtime's
/// `AuraApi::authorities()` hands a collator set back in — and so the round-robin order the
/// authorizer hash commits to.
///
/// A set written `alice,bob` in genesis comes back as `bob,alice`, because pallet-collator-selection
/// keeps its invulnerables sorted by account id and a collator's account id here is its own public
/// key. The two orders are two different authorizers, so naming a hash has to try both.
pub fn in_authority_order(names: &[&str], scheme: Scheme) -> Result<String, String> {
	let mut keyed = names
		.iter()
		.map(|name| CollatorPair::from_dev_name(scheme, name).map(|pair| (pair.public(), *name)))
		.collect::<Result<Vec<_>, _>>()?;
	keyed.sort();
	Ok(keyed.iter().map(|(_, name)| *name).collect::<Vec<_>>().join(","))
}

/// The token that rides the authorizer's sudo lane: the sentinel key, and nothing else that
/// means anything.
///
/// It is what gets a command past an authorizer with no para to match the work item against,
/// which is the only way onto a parked core. No collator set is involved — the authorizer sees
/// the key and stops looking — so the other two fields carry their shortest encodings.
pub fn sudo_token() -> Authorization {
	Authorization(AuthToken { proof: Vec::new(), key: SUDO_KEY, signature: [0u8; 64] }.encode())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn public(scheme: Scheme, name: &str) -> String {
		let pair = CollatorPair::from_dev_name(scheme, name).expect("dev names derive; qed");
		crate::format::hex(&pair.public())
	}

	/// The collator set root commits every collator's key, and the collator reads its own key
	/// from a keystore this tool never sees. If the two derivations ever diverge, every proof
	/// this tool builds is for a set the collator is not in — so pin the derivation against
	/// substrate's published dev keys rather than against ourselves, on both curves.
	#[test]
	fn dev_names_are_substrates_dev_keys_works() {
		assert_eq!(
			public(Scheme::Ed25519, "alice"),
			"88dc3417d5058ec4b4503e0c12ea1a0a89be200fe98922423d4334014fa6b0ee"
		);
		assert_eq!(
			public(Scheme::Sr25519, "alice"),
			"d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d"
		);
		// Capitalisation is the operator's convenience, not a different key.
		assert_eq!(public(Scheme::Sr25519, "Alice"), public(Scheme::Sr25519, "alice"));
		assert_ne!(public(Scheme::Sr25519, "bob"), public(Scheme::Sr25519, "alice"));
	}

	/// A queue hash commits to the whole config, so two paras on the same collator set must not
	/// collide, and the same para must hash the same way every time this tool runs. The scheme
	/// is not in the config at all — it is in the blob, and so in `code_hash`.
	#[test]
	fn each_para_gets_its_own_authorizer_works() {
		let aura = Aura::from_dev_names("alice,bob", Scheme::Sr25519, CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed");
		assert_ne!(aura.hash(ParaId(0)), aura.hash(ParaId(1)));
		assert_eq!(aura.hash(ParaId(0)), aura.hash(ParaId(0)));
		// And a different collator set is a different core, even for the same para.
		let alone = Aura::from_dev_names("alice", Scheme::Sr25519, CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed");
		assert_ne!(aura.hash(ParaId(0)), alone.hash(ParaId(0)));
	}

	/// Parking is an assignment state of its own, not the absence of one: it must not collide
	/// with any para's authorizer, and it must be reproducible, or `free-core` would write a hash
	/// the tool could never recognise again — and a parked core is only re-assignable because
	/// this tool can name what its queue holds.
	#[test]
	fn a_parked_core_has_an_authorizer_of_its_own_works() {
		let aura = Aura::from_dev_names("alice,bob", Scheme::Sr25519, CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed");
		assert!(aura.parked_config().para_ids.is_empty());
		assert_eq!(aura.parked_hash(), aura.parked_hash());
		for para in 0..8 {
			assert_ne!(aura.parked_hash(), aura.hash(ParaId(para)));
		}
		// The collator set is still committed to, so parking core A does not make it look like
		// core B parked under a different set.
		let alone = Aura::from_dev_names("alice", Scheme::Sr25519, CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed");
		assert_ne!(aura.parked_hash(), alone.parked_hash());
	}

	/// A runtime hands its collator set back sorted by account id, not in the order genesis names
	/// it, and that order is what the authorizer hash commits to — so a tool that only ever tries
	/// the written order cannot name the hash a two-collator para is actually running under.
	/// Pinned against the dev keys rather than against a second call to the same sort.
	#[test]
	fn the_authority_order_is_by_key_not_by_name_works() {
		let order = |names: &[&str]| {
			in_authority_order(names, Scheme::Sr25519).expect("dev names derive; qed")
		};
		assert_eq!(order(&["alice", "bob"]), "bob,alice");
		assert_eq!(order(&["alice", "bob", "charlie"]), "bob,charlie,alice");
		// A single collator is the case that hides the difference: any order is the right one.
		assert_eq!(order(&["alice"]), "alice");
		// The keys are the curve's, so the order can be the curve's too.
		assert_ne!(
			in_authority_order(&["alice", "bob"], Scheme::Ed25519).expect("dev names derive; qed"),
			String::new()
		);
	}

	/// The same names on a different curve are a different set, so a core assigned under one
	/// scheme authorizes nothing signed under the other even before the verifier is reached.
	#[test]
	fn the_scheme_changes_the_authorizer_works() {
		let ed = Aura::from_dev_names("alice", Scheme::Ed25519, CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed");
		let sr = Aura::from_dev_names("alice", Scheme::Sr25519, CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed");
		assert_ne!(ed.hash(ParaId(0)), sr.hash(ParaId(0)));
	}
}
