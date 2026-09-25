//! Protocol constants of the Parachain Service (spec §3.1, §5, §6.1) and the
//! Gray Paper values they derive from.

use parachain_service_core::types::Timeslot;

/// Gray Paper `C_corecount`.
pub const CORE_COUNT: usize = 341;

/// Max age (in timeslots) of a work-package's lookup-anchor — Gray Paper `L` (~24 h).
pub const MAX_LOOKUP_AGE: Timeslot = 24 * 3600 / 6;

/// Gray Paper `C_expungeperiod = C_maxlookupanchorage + 4800 = 19 200` (~32 h).
/// A preimage forgotten at timeslot `y` may only be expunged by a second
/// `forget` once `now > y + EXPUNGE_PERIOD`. See §6.1.
pub const EXPUNGE_PERIOD: Timeslot = MAX_LOOKUP_AGE + 4800;

/// §3.1 — per-parachain log byte budget (exact encoded size of all entries).
pub const PARACHAIN_LOG_BYTE_CAP: usize = 64 * 1024;

/// §3.3 — service-chosen cap on the auth-trace bytes stored in `parachain_log`.
pub const STORED_AUTH_TRACE_CAP: usize = 256;

/// §3.1 — the JAM authorizer queue holds exactly 80 hashes (Gray Paper `C_authqueuesize`).
pub const AUTHORIZER_QUEUE_LEN: usize = 80;

/// §3.1 — cap on `staged_validator_keys` (`CORE_COUNT * 3 = 1023`).
pub const MAX_STAGED_VALIDATOR_KEYS: usize = CORE_COUNT * 3;

/// §4.3 — cap on the child PVM's heap (1 GiB), the upper bound `grow_heap` can reach.
pub const MAX_PVF_HEAP_SIZE: u64 = 1 << 30;

/// §3.1 — the portion of the incoming-transfer queue Asset Hub pre-provisions in
/// its baseline. PROVISIONAL: must be derived from a benchmarked `min_memo_gas`
/// (§5.1); FIXME before production.
pub const MAX_INCOMING_TRANSFERS: usize = 1000;

/// §3.1 — cap on the transfers one `incoming_transfers` bucket may hold, so
/// Asset Hub can bound the cost of reading any single bucket however many
/// transfers arrive at once. A bucket that reaches this many is closed and the
/// next arrival opens a fresh id (§5.1).
pub const MAX_TRANSFERS_PER_BUCKET: u32 = 512;

/// §5.1 gas gate — the fixed cost of applying one work report, independent of
/// its contents. Sized for the costliest case measured: a full 64 KiB
/// `parachain_log` rewritten under a 4 KiB head. PROVISIONAL: must be
/// benchmarked; FIXME before production.
pub const REPORT_BASE_GAS: u64 = 5_000_000;

/// §5.1 gas gate — the cost of replaying one upward message and the state writes
/// it implies, charged flat per message. Covers the fixed-size variants measured
/// (the costliest caches an 80-hash `AssignCore`); variants scaling with their
/// payload or with stored state (`SetKV` values, `CleanUpBucketsUpTo`,
/// `SetValidatorKeys` against a large staging buffer) can exceed it.
/// PROVISIONAL: must be benchmarked per variant; FIXME before production.
pub const UPWARD_MESSAGE_GAS: u64 = 250_000;
