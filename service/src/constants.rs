//! Protocol constants of the Parachain Service (spec §3.1, §5, §6.1) and the
//! Gray Paper values they derive from.

use parachain_service_interface::types::Timeslot;

/// Gray Paper `C_corecount`.
pub const CORE_COUNT: usize = 341;

/// §5.2 — pending-upgrade deadline relative to current timeslot (24 h at 6 s slots).
pub const UPGRADE_TIMEOUT_TIMESLOTS: Timeslot = 24 * 3600 / 6;

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

/// §3.1 — the portion of the incoming-transfer queue Asset Hub pre-provisions in
/// its baseline. PROVISIONAL: must be derived from a benchmarked `min_memo_gas`
/// (§5.1); FIXME before production (SPEC_GAPS #2).
pub const MAX_INCOMING_TRANSFERS: usize = 1000;

/// Valid `stagingset` lengths for JAM `designate` (Gray Paper Safrole `valcount`):
/// `3 * c` for `c` in `2 ..= CORE_COUNT`.
pub fn is_valid_val_count(len: usize) -> bool {
	len % 3 == 0 && len >= 6 && len <= 3 * CORE_COUNT
}

/// Cap on the gas limit a replayed `transfer_out` may carry (D-6/D-9). The
/// destination's `min_memo_gas` is looked up at replay time; above this cap
/// the JAM `transfer` is not called and `TransferFailed` is logged. Gray Paper
/// `Ω_T` charges the transfer's full gas limit to the SENDER's meter, whose
/// whole-report budget is `Ga = 10_000_000` — so this must be a small fraction
/// of `Ga` or one hostile destination burns the entire invocation. Set to
/// `Ga / 100`: with 1 gas ≈ 1 reference-CPU cycle (polkajam benchmarks host
/// costs in cycles) that is ~30-50 µs — plenty for a memo handler doing a few
/// storage writes, while our own recording side costs ~1.6k gas. A realistic
/// destination handler (two lookup-map writes + counter increment) measures
/// ~1.7k gas per transfer (`accumulate_gas.rs::dest_handler_flood_works`), so
/// the cap could drop to `Ga / 1000` — bounding a full 345-transfer digest to
/// 0.35x `Ga` (F-13) with ~6x headroom per transfer.
/// FIXME: pick the production value and co-derive it with the per-digest
/// message caps so `count x cap ≤ margin x Ga` (SPEC_GAPS #2/#3).
pub const MAX_TRANSFER_GAS: u64 = 100_000;
