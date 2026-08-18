//! The upward-message vocabulary a PVF can emit during Refine (spec §3.3, §4.3).
//!
//! Each variant corresponds 1:1 to a side-effect host function in §4.3. Refine
//! buffers them in emission order into the work digest; Accumulate replays the
//! list in order (§5.1 step 7).

extern crate alloc;

use crate::types::{
	AuthorizerHash, Balance, CoreIndex, Hash, HeadData, Memo, ParaId, ServiceId, Timeslot,
	ValidationCodeHash, ValidatorKey, ASSET_HUB_PARA_ID, CORETIME_PARA_ID,
};
use alloc::vec::Vec;
use bounded_collections::{BoundedVec, ConstU32};
use codec::{Compact, Decode, Encode};

/// Maximum number of upward messages per Refine invocation / `ParachainWorkDigest`.
/// Spec §4.3.
pub const MAX_UPWARD_MESSAGES_PER_DIGEST: u32 = 1024;

/// Per-call cap on `set_validator_keys` chunks. Spec §4.3, §5.3.
pub const SET_VALIDATOR_KEYS_MAX_KEYS: usize = 30;

/// Upward messages emitted via host functions during Refine and replayed in order
/// by Accumulate.
pub type UpwardMessages = BoundedVec<UpwardMessage, ConstU32<MAX_UPWARD_MESSAGES_PER_DIGEST>>;

/// Payload of `transfer_out` / `UpwardMessage::TransferOut` (spec §5.1).
///
/// `source = None` means this service, matching JAM's self sentinel. The two
/// selectors choose the balance on each side — which of `source`'s is debited and
/// which of `dest`'s is credited; `true` means the supervisor balance. `deferred`
/// is `None` for a plain move and `Some((memo, gas))` for a deferred transfer;
/// the two are inseparable because JAM ignores the gas limit when no memo is
/// supplied. `id` is caller-chosen and echoed back in
/// `AccumulateLog::TransferFailed` so the parachain can match up the failure.
///
/// Doubles as the `transfer_out` host-call argument encoding: seven fields exceed
/// the six-register window, so the guest passes this SCALE-encoded (D-10). Field
/// order is the design doc's, so the two encodings are identical.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub struct TransferOutArgs {
	pub source: Option<ServiceId>,
	pub dest: ServiceId,
	pub amount: Compact<Balance>,
	pub id: Compact<u64>,
	pub source_supervisor_balance: bool,
	pub dest_supervisor_balance: bool,
	pub deferred: Option<(Memo, u64)>,
}

/// Upward messages emitted via host functions during Refine (spec §3.3).
///
/// Variant order (SCALE discriminants) follows the design doc's §3.3 listing.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum UpwardMessage {
	/// From `request_code_upgrade`: start a PVF code upgrade (§5.2).
	RequestCodeUpgrade { hash: ValidationCodeHash, len: Compact<u32> },
	/// From `solicit`: request a preimage be made available in the parachain's
	/// own preimage store (§6.1).
	Solicit { hash: Hash, len: Compact<u32> },
	/// From `forget`: release a previously solicited preimage (§6.1). `para_id`
	/// names whose reference is released; a para may only name itself, while the
	/// Coretime chain may name any para (§6.4).
	Forget { para_id: ParaId, hash: Hash, len: Compact<u32> },
	/// From `kv_set`: upsert `key_value_storage[(para_id, key)] = value` (§6.1).
	SetKV { key: Vec<u8>, value: Vec<u8> },
	/// From `kv_remove`: remove `key_value_storage[(para_id, key)]` (§6.1). Same
	/// `para_id` delegation rule as `Forget`.
	RemoveKV { para_id: ParaId, key: Vec<u8> },
	/// From `transfer_out`: move balance between JAM services (Asset Hub only, §5.1).
	TransferOut(TransferOutArgs),
	/// From `assign_core`: schedule a core's `assign` — queue + assigner,
	/// written atomically (Coretime only, §7.1).
	///
	/// An empty `queue` cancels any cached entry for the core (no JAM call).
	/// `new_assigner = None` keeps this service as the core's assigner;
	/// `Some(s)` hands the core to `s` (one-way).
	AssignCore {
		core: CoreIndex,
		queue: Vec<AuthorizerHash>,
		new_assigner: Option<ServiceId>,
		jam_slot: Timeslot,
	},
	/// From `set_validator_keys`: append a chunk of upcoming validator keys
	/// (Asset Hub only, §5.3).
	SetValidatorKeys { keys: Vec<ValidatorKey>, is_last: bool },
	/// From `consume_transfers_up_to`: drop every queued transfer bucket up to
	/// and including this slot (Asset Hub only, §5.1).
	ConsumeTransfersUpTo(Timeslot),
	/// From `parachain_service_upgrade`: replace the Parachain Service's own
	/// service code (Asset Hub only, §5.4).
	UpgradeService { code_hash: Hash, len: Compact<u32>, min_acc_gas: u64, min_memo_gas: u64 },
	/// From `parachain_set_head`: upsert a parachain's head data (Coretime only, §6).
	ParachainSetHead { para_id: ParaId, new_head: HeadData },
	/// From `parachain_set_validation_code`: upsert a parachain's validation
	/// code, bypassing the normal upgrade lifecycle (Coretime only, §6).
	ParachainSetValidationCode {
		para_id: ParaId,
		new_validation_code_hash: ValidationCodeHash,
		new_validation_code_len: Compact<u32>,
	},
	/// From `parachain_clean_up`: remove all per-parachain state (Coretime only, §6.4).
	ParachainCleanUp(ParaId),
	/// From `parachain_set_state_balance`: overwrite a parachain's total state
	/// balance (Coretime only, §6.1).
	ParachainSetStateBalance { para_id: ParaId, new_total: Compact<Balance> },
}

impl UpwardMessage {
	/// Whether the host function backing this message is restricted to Asset Hub
	/// (spec §4.3).
	pub fn is_asset_hub_only(&self) -> bool {
		matches!(
			self,
			Self::TransferOut { .. } |
				Self::SetValidatorKeys { .. } |
				Self::ConsumeTransfersUpTo(_) |
				Self::UpgradeService { .. }
		)
	}

	/// Whether the host function backing this message is restricted to the
	/// Coretime chain (spec §4.3, §6).
	pub fn is_coretime_only(&self) -> bool {
		matches!(
			self,
			Self::AssignCore { .. } |
				Self::ParachainSetHead { .. } |
				Self::ParachainSetValidationCode { .. } |
				Self::ParachainCleanUp(_) |
				Self::ParachainSetStateBalance { .. }
		)
	}

	/// Per §4.3: a host function taking a `para_id` aborts unless it names the
	/// calling parachain — except from the Coretime chain, which may name any
	/// para (§6.4). `true` iff this message names a para other than `origin`
	/// without that right.
	pub fn targets_foreign_para(&self, origin: ParaId) -> bool {
		if origin == CORETIME_PARA_ID {
			return false;
		}
		match self {
			Self::Forget { para_id, .. } | Self::RemoveKV { para_id, .. } => *para_id != origin,
			_ => false,
		}
	}

	/// Per §4.3: host functions restricted to a specific parachain (Asset Hub or
	/// the Coretime chain) abort when invoked from any other parachain, as does
	/// naming a foreign `para_id` without the right to. `true` iff `origin` may
	/// emit this message.
	pub fn allowed_for(&self, origin: ParaId) -> bool {
		!(self.is_asset_hub_only() && origin != ASSET_HUB_PARA_ID) &&
			!(self.is_coretime_only() && origin != CORETIME_PARA_ID) &&
			!self.targets_foreign_para(origin)
	}
}
