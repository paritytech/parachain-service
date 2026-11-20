//! The per-parachain log ABI (spec §3.1, §5.1) and the constants it is sized by.
//!
//! These types are written by the service's Accumulate and read back by a
//! parachain's own validation code through the JAM state proof (the feedback
//! channel of §5.1). They live in the shared interface crate so both sides
//! decode the exact same bytes: the service's storage codec and the runtime's
//! reader must agree on every discriminant.
//!
//! Pure eviction/pruning logic and the storage accessors stay in the service
//! crate; only the wire types and their sizing constants live here.

extern crate alloc;

use alloc::vec::Vec;
use bounded_collections::{BoundedVec, ConstU32};
use codec::{Compact, Decode, Encode};

use crate::{
	types::{Balance, CoreIndex, Hash, ParaId, ServiceId, Timeslot, ValidationCodeHash},
	upward_message::Target,
};

/// §3.1 — per-parachain log byte budget (exact encoded size of all entries).
pub const PARACHAIN_LOG_BYTE_CAP: usize = 64 * 1024;

/// §3.3 — service-chosen cap on the auth-trace bytes stored in `parachain_log`.
pub const STORED_AUTH_TRACE_CAP: usize = 256;

/// The maximum byte length of a `report_error` payload. Spec §4.3.
pub const MAX_REPORT_ERROR_PAYLOAD: u32 = 1024;

/// Why a state-balance reservation failed (spec §3.1, §6.1).
///
/// Covers solicit/preimage and `kv_set` growth. Baseline-covered writes have
/// no per-write allowance check and do not emit this log on host failure.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum InsufficientBalanceReason {
	/// A `solicit` of the preimage with `hash` and `len`.
	Solicit { hash: Hash, len: Compact<u32> },
	/// A `kv_set(key, value)` write. Only the hash of `key` is recorded so an
	/// arbitrarily large user key cannot inflate `parachain_log`.
	SetKV { key_hash: Hash },
}

/// Why a JAM `transfer` replaying a `TransferOut` failed (spec §5.1 step 6).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum TransferError {
	/// `source` is not a known service.
	UnknownSource,
	/// `dest` is not a known service.
	UnknownDestination,
	/// The service is not `source`'s effective supervisor; only its own regular
	/// balance is exempt. Takes precedence over `DestinationNotSupervised`.
	SourceNotSupervised,
	/// A plain move to another service needs the service to be `dest`'s effective
	/// supervisor. Also covers an identity write (`source == dest` with both
	/// selectors equal).
	DestinationNotSupervised,
	/// The supplied gas is below `dest`'s `min_memo_gas`.
	GasBelowDestinationMinimum,
	/// The `source` service cannot cover a `TransferAmount::Exact`, either because
	/// the debited balance is too small or because the transfer would leave it
	/// below its threshold balance.
	InsufficientServiceBalance,
}

/// Why a `Service`-targeted `solicit` failed (spec §6.5).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ServiceSolicitError {
	/// The named service does not exist.
	UnknownService,
	/// The Parachain Service is not its effective supervisor.
	NotSupervised,
	/// The request would leave the target below its threshold balance.
	TargetCannotAfford,
	/// The target already has a live request for this preimage that is not
	/// awaiting re-solicitation.
	AlreadySolicited,
}

/// Why an `eject_service` failed (spec §6.5).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ServiceEjectError {
	/// The named service does not exist.
	UnknownService,
	/// The Parachain Service is not its effective supervisor.
	NotSupervised,
	/// The service still holds storage or preimage requests.
	NotEmpty,
	/// The service was created in this timeslot.
	CreatedThisSlot,
	/// The Parachain Service named itself.
	TargetIsSelf,
}

/// Why a `set_service_supervisor` failed (spec §6.5).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ServiceSupervisorError {
	/// The named service does not exist.
	UnknownService,
	/// The proposed new supervisor does not exist.
	UnknownNewSupervisor,
	/// The Parachain Service is not its effective supervisor.
	NotSupervised,
}

/// How a `create_service` turned out (spec §6.5). The only §6.5 operation that
/// logs a success, since the caller needs the assigned id announced to it.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum ServiceCreationResult {
	/// Succeeded, carrying the id JAM assigned.
	Created(ServiceId),
	/// The Parachain Service cannot fund the new service.
	CannotAfford,
	/// A `desired_id` in the protected range that is already in use.
	IdTaken,
}

/// Why a Coretime state-balance update was rejected (§6.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum StateBalanceRejection {
	BelowUsed { current_total: Compact<Balance>, current_used: Compact<Balance> },
	ParachainIsDeregistering,
}

/// Structured reason a `refine` invocation failed.
///
/// Every variant is raised only after §4.1 step 2 fixes an authoritative
/// `para_id`, since the entry lands in `parachain_log[para_id]`. Failures before
/// that panic instead (§4.2).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum RefineLog {
	/// `historical_lookup(validation_code)` returned `None`: the
	/// validation code preimage is not available in the service's store
	/// at the lookup-anchor. See §4.1 step 4.
	ValidationCodeLookupFailed,
	/// Opaque payload supplied by the PVF via `report_error(data)` before
	/// failing the execution (max 1024 bytes).
	Opaque(BoundedVec<u8, ConstU32<1024>>),
	/// A `SetValidatorKeys` was sent more than once in a single Refine
	/// invocation. See §4.3, §5.3.
	SetValidatorKeysRepeated,
	/// A `SetValidatorKeys` chunk carried more than 30 keys. See §4.3, §5.3.
	TooManyValidatorKeys,
	/// The PVF emitted more than `MAX_UPWARD_MESSAGES_PER_DIGEST` upward
	/// messages in a single Refine invocation. See §4.3.
	TooManyUpwardMessages,
	/// The parachain's 40 KiB upward-message budget was exceeded. See §4.3.
	UpwardMessagesTooLarge,
	/// The PVF invoked a host function restricted to another parachain
	/// (Asset Hub or the Coretime chain). See §4.3.
	RestrictedHostFunction,
	/// An `assign_core` queue outside 1..=`AUTHORIZER_QUEUE_LEN` hashes — and any
	/// queue other than exactly `AUTHORIZER_QUEUE_LEN` when handing the core to a
	/// new assigner. See §4.3.
	InvalidAuthorizerQueue,
	/// The encoded `ParachainWorkDigest` (head data + upward messages) would
	/// exceed the Gray Paper's 48 KiB combined result-blob + auth-trace
	/// budget. See §4.1.
	RefineOutputTooLarge,
	/// The PVF did not call `set_parent_head_hash` or `set_head` exactly once.
	/// Both head declarations are mandatory. See §4.2.
	InvalidHeadDeclaration,
	/// `set_head` was called with head data beyond the 4 KiB `HeadData` bound.
	/// See §4.3.
	HeadDataTooLarge,
	/// An `AssignCore` named a core at or above `C_corecount`. See §3.3.
	InvalidCoreIndex,
	/// A `SetKV` or `RemoveKV` carried an empty key, or a `SetKV` an empty value.
	/// See §3.3.
	EmptyKVKeyOrValue,
}

/// Events recorded while Accumulating for a parachain (spec §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum AccumulateLog {
	/// Available state balance insufficient for the operation. Spec §6.1.
	InsufficientStateBalance { reason: InsufficientBalanceReason },
	/// `ParachainSetStateBalance` rejected for the named target and reason.
	StateBalanceUpdateRejected {
		para_id: ParaId,
		attempted: Compact<Balance>,
		reason: StateBalanceRejection,
	},
	/// JAM `designate` rejected the assembled key set, because its length is not
	/// in `valcount` or this service is not the delegator. The staging buffer is
	/// cleared regardless. Spec §5.3.
	DesignateRejected { len: Compact<u32> },
	/// A `set_validator_keys` chunk would overflow `staged_validator_keys`;
	/// the append is rejected. Spec §5.3.
	StagedValidatorKeysOverflow,
	/// An `UpgradeService` whose code Asset Hub does not reference, or that is
	/// not available for lookup. Spec §5.4.
	ServiceUpgradePreimageMissing { code_hash: Hash },
	/// An `Announcement` whose code is not usable (§5.2).
	CodeUpgradeNotAvailable { hash: Hash, len: Compact<u32> },
	/// An `Apply` that does not match the standing announcement (§5.2).
	CodeUpgradeNotAnnounced { hash: Hash, len: Compact<u32> },
	/// A `Forget` naming code in use: the para's active or announced validation
	/// code (§5.2), or for Asset Hub the Parachain Service's active code (§5.4).
	CanNotRemoveCode { hash: Hash, len: Compact<u32> },
	/// JAM rejected an `assign` because this service is no longer the core's
	/// assigner (§7.1).
	CoreNotAssignable { core: CoreIndex },
	/// The JAM `transfer` replaying a `TransferOut` failed. `id` is the
	/// caller-supplied identifier, echoed back so the parachain can match the
	/// failure to its own record. Spec §5.1 step 6.
	TransferFailed { id: Compact<u64>, error: TransferError },
	/// The `TransferOut` with this `id` and a `TransferAmount::Max` went through,
	/// moving `amount`. Spec §5.1.
	TransferredAmount { id: Compact<u64>, amount: Compact<Balance> },
	/// A `forget` removed the last referencer without expunging the preimage;
	/// `forget` again once strictly past `due`. Spec §6.1.
	ForgetAgainAt { target: Target, hash: Hash, len: Compact<u32>, due: Timeslot },
	/// A `Service`-targeted `forget` named a preimage the service never
	/// requested. Spec §6.5.
	ServiceForgetNotRequested { service: ServiceId, hash: Hash, len: Compact<u32> },
	/// A `remove_service_storage` named a key the service does not hold;
	/// `key_hash` is the hash of that key. Spec §6.5.
	ServiceKeyNotFound { service: ServiceId, key_hash: Hash },
	/// A `Service`-targeted `solicit` failed. Spec §6.5.
	ServiceSolicitFailed { service: ServiceId, error: ServiceSolicitError },
	/// An `eject_service` failed. Spec §6.5.
	ServiceEjectFailed { service: ServiceId, error: ServiceEjectError },
	/// An `eject_service` succeeded, crediting the service's regular plus
	/// supervisor balance back. Spec §6.5.
	ServiceEjected { service: ServiceId, credited: Compact<Balance> },
	/// A `set_service_supervisor` failed. Spec §6.5.
	ServiceSupervisorFailed { service: ServiceId, error: ServiceSupervisorError },
	/// Announces a `create_service` outcome, echoing the caller's own `message_id`.
	/// Spec §6.5.
	ServiceCreation { message_id: Compact<u64>, result: ServiceCreationResult },
	/// `parachain_clean_up` rejected: state held beyond baseline + codes. Spec §6.4.
	TooMuchStateHeld,
	/// The candidate's code does not match the active code. §5.1.
	/// Appended to preserve the SCALE indices of existing events.
	InvalidCodeHash { hash: ValidationCodeHash },
}

/// The auth trace stored with a Refine failure, truncated to 256 bytes (§3.3).
pub type StoredAuthTrace = BoundedVec<u8, ConstU32<{ STORED_AUTH_TRACE_CAP as u32 }>>;

/// One log entry (spec §3.1). Each carries its timeslot inline in the log vector.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum LogEntry {
	/// What went wrong during Refine, with the work-report's auth trace.
	Refine { error: RefineLog, auth_trace: StoredAuthTrace },
	/// Events recorded while Accumulating a work digest.
	Accumulate { entries: Vec<AccumulateLog> },
}

/// A parachain's full log value: `(timeslot, entry)` pairs in append order.
pub type ParachainLog = Vec<(Timeslot, LogEntry)>;

/// The `AccumulateLog` events of the most recent `Accumulate` entry, if any.
///
/// A parachain reads its log to learn why its last candidates were rejected
/// (§5.1); the newest batch is the one that speaks about the newest work
/// package, which is why the walk runs from the back.
pub fn latest_accumulate(log: &ParachainLog) -> Option<&[AccumulateLog]> {
	log.iter().rev().find_map(|(_, entry)| match entry {
		LogEntry::Accumulate { entries } => Some(entries.as_slice()),
		LogEntry::Refine { .. } => None,
	})
}

/// Whether any entry of `log` carries `event`, in any batch.
pub fn contains_accumulate(log: &ParachainLog, event: &AccumulateLog) -> bool {
	log.iter().any(|(_, entry)| match entry {
		LogEntry::Accumulate { entries } => entries.iter().any(|e| e == event),
		LogEntry::Refine { .. } => false,
	})
}
