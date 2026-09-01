//! The AURA authorizer a package runs under: the collator set, the config it commits to, and the
//! token that satisfies it.
//!
//! The set is named, never supplied: `--collators alice,bob` derives the same `//Name` ed25519
//! keys the harness puts in a collator's keystore, so nothing here needs key material on disk.
//! Everything else — the trie, the leaf hashing, the signing payload — comes out of
//! `parachain-authorizer`, the crate the guest is built from, because a queue hash commits to the
//! whole config and a hash nobody can reproduce is a core nobody can use.

use codec::{DecodeAll as _, Encode as _};
use jam_types::{
	AuthConfig as RawAuthConfig, Authorization, Authorizer, AuthorizerHash, CodeHash, ServiceId,
	WorkPackage,
};
use parachain_authorizer::aura::{
	build_collator_tree, expected_collator_index, signable_work_package_hash, AuthConfig,
	AuthToken, Command,
};
use parachain_service_interface::types::ParaId;
use primitive_types::H256;
use sp_core::{ed25519, Pair as _};

/// A para's AURA authorizer, and the keys needed to author under it.
pub struct Aura {
	/// Code hash of the deployed authorizer blob. The queue holds `blake2b(code_hash ‖ config)`,
	/// so this and the config together are the core's identity.
	pub code_hash: CodeHash,
	/// The JAM service the authorizer admits work items for, and nothing else.
	pub service: ServiceId,
	/// Length of a para slot in JAM timeslots; the round-robin's divisor.
	pub slot_duration: u32,
	pairs: Vec<ed25519::Pair>,
	root: H256,
	proofs: Vec<Vec<H256>>,
}

impl Aura {
	/// Derive the collator set from dev names, in the order the round-robin walks it.
	pub fn from_dev_names(
		names: &str,
		code_hash: CodeHash,
		service: ServiceId,
		slot_duration: u32,
	) -> Result<Self, String> {
		let pairs = names
			.split(',')
			.map(|name| dev_pair(name.trim()))
			.collect::<Result<Vec<_>, _>>()?;
		if pairs.is_empty() {
			return Err("--collators names at least one collator".to_string());
		}
		let keys: Vec<[u8; 32]> = pairs.iter().map(|pair| pair.public().0).collect();
		let (root, proofs) = build_collator_tree(&keys);
		Ok(Self { code_hash, service, slot_duration, pairs, root, proofs })
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

	pub fn authorizer(&self, para: ParaId) -> Authorizer {
		Authorizer { code_hash: self.code_hash, config: RawAuthConfig(self.config(para).encode()) }
	}

	/// What a core's authorizer queue holds when it is running `para`.
	pub fn hash(&self, para: ParaId) -> AuthorizerHash {
		self.authorizer(para).hash(jam_std_common::hash_raw)
	}

	/// Sign `package` as whichever collator its lookup anchor names, optionally carrying a
	/// core-assignment command.
	///
	/// The round-robin index is read out of the config the package itself carries, not out of
	/// this struct's fields, so the token can only ever be built against the config the
	/// authorizer is going to read. Rather than hunt for an anchor that names a particular
	/// collator, this signs as the one the anchor already names — every dev key is to hand.
	pub fn token(
		&self,
		package: &WorkPackage,
		command: Option<Command>,
	) -> Result<Authorization, String> {
		let config = AuthConfig::decode_all(&mut &package.authorizer.config[..])
			.map_err(|e| format!("the package's own authorizer config does not decode: {e}"))?;
		let index = expected_collator_index(package.context.lookup_anchor_slot, &config) as usize;
		let pair = self
			.pairs
			.get(index)
			.ok_or_else(|| format!("the collator set has no member {index}"))?;

		let payload = AuthToken::signing_payload(signable_work_package_hash(package), &command);
		let token = AuthToken {
			proof: self.proofs[index].clone(),
			key: pair.public().0,
			signature: pair.sign(payload.as_bytes()).0,
			control_command: command,
		};
		println!(
			"signing as collator {index} of {} (lookup anchor slot {})",
			self.pairs.len(),
			package.context.lookup_anchor_slot
		);
		Ok(Authorization(token.encode()))
	}
}

/// The ed25519 key behind a dev name, derived exactly as `key insert --suri //Name` would.
fn dev_pair(name: &str) -> Result<ed25519::Pair, String> {
	let mut capitalized = name.to_string();
	capitalized[..1].make_ascii_uppercase();
	ed25519::Pair::from_string(&format!("//{capitalized}"), None)
		.map_err(|e| format!("no dev key for collator {name:?}: {e:?}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The collator set root commits every collator's key, and the collator reads its own key
	/// from a keystore this tool never sees. If the two derivations ever diverge, every proof
	/// this tool builds is for a set the collator is not in — so pin the derivation against
	/// substrate's published dev key rather than against ourselves.
	#[test]
	fn dev_names_are_substrates_dev_keys_works() {
		let alice = dev_pair("alice").expect("//Alice derives; qed");
		assert_eq!(
			crate::format::hex(&alice.public().0),
			"88dc3417d5058ec4b4503e0c12ea1a0a89be200fe98922423d4334014fa6b0ee"
		);
		// Capitalisation is the operator's convenience, not a different key.
		assert_eq!(dev_pair("Alice").expect("//Alice derives; qed").public(), alice.public());
		assert_ne!(dev_pair("bob").expect("//Bob derives; qed").public(), alice.public());
	}

	/// A queue hash commits to the whole config, so two paras on the same collator set must not
	/// collide, and the same para must hash the same way every time this tool runs.
	#[test]
	fn each_para_gets_its_own_authorizer_works() {
		let aura = Aura::from_dev_names("alice,bob", CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed");
		assert_ne!(aura.hash(ParaId(0)), aura.hash(ParaId(1)));
		assert_eq!(aura.hash(ParaId(0)), aura.hash(ParaId(0)));
		// And a different collator set is a different core, even for the same para.
		let alone =
			Aura::from_dev_names("alice", CodeHash::zero(), 5, 1).expect("dev names derive; qed");
		assert_ne!(aura.hash(ParaId(0)), alone.hash(ParaId(0)));
	}
}
