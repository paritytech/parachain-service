//! The parachain service's genesis description.

use std::collections::{BTreeMap, BTreeSet};

use codec::Encode;
use jam_chainspec::GenesisService;
use jam_types::{AuthorizerHash, Balance, ServiceId};
use parachain_service::{
	state::{
		para_info::{ParaInfo, ValidationCode},
		preimage_registry::PreimageEntry,
		storage_key, Tag,
	},
	state_balance::{baseline_for, preimage_footprint},
	work_digest::validation_code_hash,
};
use parachain_service_interface::types::{HeadData, ParaId, ValidationCodeRef};

use crate::{parachain::ParachainSpec, Error};

/// Builder for the parachain service's [`GenesisService`]: one service entry of a
/// [`ChainSpecConfig`](jam_chainspec::ChainSpecConfig).
///
/// `build` writes the service's own per-parachain records as storage entries in
/// exactly the layout `parachain_service` reads back (spec §3.1), hosts every
/// validation-code and authorizer verifier blob as a preimage of the service, and
/// fills the cross-parachain preimage registry (§3.1, §6.1) — the state a completed
/// §6.2 registration would leave behind. The authorizer hashes end up in
/// [`Self::authorizer_hashes`], to be placed in the cores' queues of the chain
/// spec.
#[derive(Clone, Debug)]
pub struct ParachainServiceSpec {
	id: ServiceId,
	code: Vec<u8>,
	balance: Option<Balance>,
	paras: Vec<ParachainSpec>,
	extra_preimages: Vec<Vec<u8>>,
}

impl ParachainServiceSpec {
	/// The parachain service installed under `id`. The service's own code is hosted
	/// as its preimage by `jam_chainspec` itself, so it is never repeated in
	/// `preimages`.
	pub fn new(id: ServiceId, code: impl Into<Vec<u8>>) -> Self {
		Self {
			id,
			code: code.into(),
			balance: None,
			paras: Vec::new(),
			extra_preimages: Vec::new(),
		}
	}

	/// Set the service's starting balance. Defaults to `Balance::MAX`, which covers
	/// any genesis footprint.
	pub fn balance(mut self, balance: Balance) -> Self {
		self.balance = Some(balance);
		self
	}

	/// Register a parachain with the service.
	pub fn parachain(mut self, para: ParachainSpec) -> Self {
		self.paras.push(para);
		self
	}

	/// Host `blob` as an additional preimage of the service, on top of the blobs the
	/// parachains imply. Deduplicated against those: a blob already hosted is not
	/// hosted again.
	pub fn preimage(mut self, blob: impl Into<Vec<u8>>) -> Self {
		self.extra_preimages.push(blob.into());
		self
	}

	/// The authorizer hash each para's core queue must hold — one entry per para
	/// that got an [`ParachainSpec::authorizer`](ParachainSpec::authorizer) — for
	/// `ChainSpecConfig::auth_queues`.
	pub fn authorizer_hashes(&self) -> BTreeMap<ParaId, AuthorizerHash> {
		self.paras
			.iter()
			.filter_map(|p| p.authorizer.as_ref().map(|(_, hash)| (p.id, *hash)))
			.collect()
	}

	/// Build the service's genesis description.
	///
	/// Fails when a para's head exceeds the 4 KiB bound or the same para id is
	/// registered twice; both are programming errors that would otherwise corrupt
	/// the state layout silently.
	pub fn build(self) -> Result<BuiltParachainService, Error> {
		// `Balance::MAX` when unset — the unlimited default the suite relies on — spelled
		// explicitly because jam-chainspec's own `new` default is `MinimumPlus(0)`.
		let balance = self.balance.unwrap_or(Balance::MAX);
		let mut service = GenesisService::new(self.code).balance(balance);

		// Deterministic output: paras run in ParaId order, whatever the insertion
		// order, and two specs for one id are a bug — the later `ParaInfo` row would
		// overwrite the earlier one while the registry kept both referencers.
		let mut paras = self.paras;
		paras.sort_by_key(|p| p.id);
		for pair in paras.windows(2) {
			if pair[0].id == pair[1].id {
				return Err(Error::DuplicateParaId(pair[0].id.0));
			}
		}

		// Each distinct blob is hosted once; the registry entry then accumulates
		// every para referencing that `(hash, len)`.
		let mut preimages: Vec<Vec<u8>> = Vec::new();
		let mut hosted: BTreeSet<Vec<u8>> = BTreeSet::new();
		let mut registry: BTreeMap<([u8; 32], u32), BTreeSet<ParaId>> = BTreeMap::new();

		for para in &paras {
			let validation_code = para.code.as_ref().map(|code| {
				let cref =
					ValidationCodeRef { hash: validation_code_hash(code), len: code.len() as u32 };
				host(&mut preimages, &mut hosted, code);
				registry.entry((cref.hash.0, cref.len)).or_default().insert(para.id);
				ValidationCode { code_ref: cref, pinned: false }
			});
			// An authorizer's verifier blob is a preimage too — a work package can
			// name the service as its `auth_code_host` — but it names no registry
			// entry: only validation codes are referenced by `ParaInfo`.
			if let Some((code, _)) = &para.authorizer {
				host(&mut preimages, &mut hosted, code);
			}

			let head = para.head.clone().unwrap_or_default();
			let head_data: HeadData = head.try_into().map_err(|head: Vec<u8>| {
				Error::HeadDataTooLarge { para: para.id.0, len: head.len() }
			})?;

			// A para without a validation code has no preimage footprint; one with
			// one pays the full sole-user footprint whatever else references it
			// (§6.1). The registered base (`baseline_for`) covers Asset Hub's
			// service-global reservation.
			let used_state_balance = baseline_for(para.id) +
				validation_code.as_ref().map_or(0, |vc| preimage_footprint(vc.code_ref.len));
			let info = ParaInfo {
				head_data,
				validation_code,
				pending_upgrade: None,
				total_state_balance: para.total.unwrap_or(Balance::MAX),
				used_state_balance,
				is_deregistering: false,
			};
			service = service.storage(storage_key(Tag::Parachains, &para.id), info.encode());
		}

		for ((hash, len), referencers) in &registry {
			let entry = PreimageEntry { referencers: referencers.clone() };
			service =
				service.storage(storage_key(Tag::PreimageRegistry, &(*hash, *len)), entry.encode());
		}

		for blob in &self.extra_preimages {
			host(&mut preimages, &mut hosted, blob);
		}
		for blob in preimages {
			service = service.preimage(blob);
		}
		Ok(BuiltParachainService {
			id: self.id,
			balance,
			storage: service.storage.into_iter().map(|(key, value)| (key.0, value.0)).collect(),
			preimages: service
				.preimages
				.into_iter()
				.map(|blob| blob.read().expect("built blobs are bytes, not paths; qed"))
				.collect(),
		})
	}
}

/// The built service, spelled the way the suite's genesis writer consumes it: jam-chainspec
/// carries the service under `services[id]`, and the overrides writer reads the balance,
/// storage and preimages straight off the built value.
#[derive(Debug, Clone)]
pub struct BuiltParachainService {
	/// The service's id.
	pub id: ServiceId,
	/// The starting balance: `Balance::MAX` unless [`ParachainServiceSpec::balance`] said
	/// otherwise.
	pub balance: Balance,
	/// The service's storage entries, raw key and value bytes.
	pub storage: BTreeMap<Vec<u8>, Vec<u8>>,
	/// The blobs the service hosts as preimages, in insertion order.
	pub preimages: Vec<Vec<u8>>,
}

/// Add `blob` to `preimages` unless some already-hosted blob has the same bytes.
fn host(preimages: &mut Vec<Vec<u8>>, hosted: &mut BTreeSet<Vec<u8>>, blob: &[u8]) {
	if hosted.insert(blob.to_vec()) {
		preimages.push(blob.to_vec());
	}
}
