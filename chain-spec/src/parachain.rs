//! Description of one registered parachain.

use codec::Encode;
use jam_std_common::hash_raw;
use jam_types::{AuthConfig, Authorizer, AuthorizerHash};
use parachain_authorizer::aura::AuthConfig as AuraConfig;
use parachain_service_interface::types::{Balance, ParaId};

/// One parachain registered with the service at genesis (spec §6.2).
///
/// Every aspect but the id is optional: a para registered without a
/// [`validation_code`](Self::validation_code) is a valid registration still waiting
/// for its preimage, and a para without a [`state_balance`](Self::state_balance)
/// has unlimited headroom (`Balance::MAX`), mirroring the
/// [`GenesisService`](jam_chainspec::GenesisService) default.
#[derive(Clone, Debug)]
pub struct ParachainSpec {
	pub(crate) id: ParaId,
	pub(crate) head: Option<Vec<u8>>,
	/// Validation code (PVF blob), hosted as a preimage of the service and named by
	/// the para's `ParaInfo`.
	pub(crate) code: Option<Vec<u8>>,
	pub(crate) total: Option<Balance>,
	/// The authorizer's verifier blob and the hash its core queue must hold.
	pub(crate) authorizer: Option<(Vec<u8>, AuthorizerHash)>,
}

impl ParachainSpec {
	/// A parachain registered under `id`, with no head, no validation code and
	/// unlimited state balance.
	pub fn new(id: ParaId) -> Self {
		Self { id, head: None, code: None, total: None, authorizer: None }
	}

	/// The head data the para starts with. The 4 KiB head-data bound is checked
	/// by [`ParachainServiceSpec::build`], not here.
	pub fn head_data(mut self, head: impl Into<Vec<u8>>) -> Self {
		self.head = Some(head.into());
		self
	}

	/// Set the para's active validation code. The blob is hosted as a preimage of
	/// the service — one per distinct blob, however many paras share it — and the
	/// para is recorded as a referencer of it in the preimage registry.
	pub fn validation_code(mut self, code: impl Into<Vec<u8>>) -> Self {
		self.code = Some(code.into());
		self
	}

	/// Set the para's total state balance (§6.1). Defaults to `Balance::MAX`, so an
	/// unmanaged para never hits the §6.1 headroom check at genesis.
	pub fn state_balance(mut self, total: Balance) -> Self {
		self.total = Some(total);
		self
	}

	/// Authorize the para's core with the verifier coded by `code` and configured by
	/// `config`.
	///
	/// `code` is hosted as a preimage of the service, deduplicated against every
	/// other blob the service hosts, so a work package can name the parachain
	/// service as its `auth_code_host`. The para contributes
	/// `blake2b-256(code_hash ‖ SCALE(config))` to
	/// [`ParachainServiceSpec::authorizer_hashes`].
	pub fn authorizer(mut self, code: impl Into<Vec<u8>>, config: &AuraConfig) -> Self {
		let code = code.into();
		let hash = authorizer_hash(&code, config);
		self.authorizer = Some((code, hash));
		self
	}

	/// The id this para is registered under.
	pub fn id(&self) -> ParaId {
		self.id
	}
}

/// `blake2b-256(code_hash ‖ SCALE(config))`, the authorizer-hash contract.
///
/// Computed through `jam_types::Authorizer::hash` so it can never drift from the
/// definition the rest of the tree uses (see `cumulus::authorizer`); the raw
/// concatenation, with no SCALE wrapper around the pair, is what makes the
/// concat argument.
fn authorizer_hash(code: &[u8], config: &AuraConfig) -> AuthorizerHash {
	let authorizer =
		Authorizer { code_hash: hash_raw(code).into(), config: AuthConfig(config.encode()) };
	authorizer.hash(hash_raw)
}
