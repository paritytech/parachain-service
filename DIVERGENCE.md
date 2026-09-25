# Model–Implementation Divergences

Places where the Rust implementation and the
[Quint spec](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/) disagree on
observable behaviour or on a derived constant. Found by reading both sides and by trace replay;
checked against spec pin `73d4d9eb9d8`.

Scope: this file covers **Quint model vs Rust**. Two neighbouring documents cover the
neighbouring questions, and entries here cross-reference them rather than restating them:

- [DECISIONS.md](./DECISIONS.md) — where the *design doc* under-specifies and the PoC had to
  choose (`D-n`), plus spec issues found while implementing (`F-n`).
- [QUINT_REPLAY.md](./QUINT_REPLAY.md) — the trace-replay harness and its normalization
  ledger.

Format: **M-n** — what differs, which side is right, what to do. Numbering is not reused, and
is disjoint from `D-n`/`F-n` so every `[DFM]-N` citation in the tree resolves to exactly one
place. Quint paths below are relative to
`vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/`.

Direction, unless an entry says otherwise: **Quint is the oracle and Rust must match it.**
Entries that invert this — the model is wrong and the finding goes upstream — say so.

---

## M-2: `TransferOut` shapes the host cannot express

**Neither is wrong; the host is behind both.**

Since Quint `148fbfb7856` the model replays a `TransferOut` as JAM's `transfer`
(`quint/foreign_services.qnt`, `transferOut`) and logs `TransferFailed` for every refusal, in
JAM's order: `UnknownSource`, `UnknownDestination`, `SourceNotSupervised`,
`DestinationNotSupervised`, `GasBelowDestinationMinimum`, `InsufficientServiceBalance`.
`service/src/accumulate/transfers.rs` refuses in the same order, and matches the model wherever
the vendored host can do what the model does (`accumulate_transfers.rs`,
`transfer_out_refusal_order_errors`).

The host has no supervision and one balance per service ([DECISIONS.md](./DECISIONS.md) D-11),
so Rust controls only this service and treats its supervisor balance as always empty. The model
differs for:

- a `source`, or a plain move's `dest`, that this service supervises in the model because it
  created it (M-11): the model moves the funds; Rust logs `SourceNotSupervised` or
  `DestinationNotSupervised`;
- a non-zero credit to a supervisor balance, including a plain move from this service's regular
  balance into its own supervisor balance: the model credits it; Rust logs
  `DestinationNotSupervised`;
- spending this service's supervisor balance after the model has funded it (through the moves
  above, or an incoming transfer to it, M-12): the model spends it; Rust logs
  `InsufficientServiceBalance`.

The model also approximates this service's threshold balance from above (it bills each
parachain as the sole user of shared entries), so close to the threshold it can refuse a
spend JAM allows. The replay harness cannot replay `TransferOut` yet.

## M-3: the model drops `ForgetAgainAt` on the code-upgrade paths — resolved

The two-phase §5.2 lifecycle has no reap and no activation release: `Apply`, a superseding
`Announcement` and, since Quint `71e422dc6d`, `ParachainSetValidationCode` all leave the
displaced code referenced until the parachain forgets it. No code-upgrade path calls
`removeReferencer` any more, so there is no `ForgetAgainAt` left to drop.

## M-4: Refine reports a different `RefineLog` variant for the same PVF

**Neither is wrong; the two orderings need reconciling.**

`quint/refine.qnt:245-275` scans the *finished* upward-message list in a fixed priority:
message count → `set_validator_keys` repetition and chunk size → assign queues → assign core
indices → parachain restrictions → 40 KiB message budget → output size → head declarations. Rust has no such scan —
the checks live in the host-call dispatcher and abort at the first offending call in **emission
order** (`ExecutorState::push` in `service/src/pvf/executor.rs`).

A non-Coretime para emitting `[TransferOut, AssignCore { queue: [] }]` logs
`InvalidAuthorizerQueue` in the model (queues are checked before restrictions) and
`RestrictedHostFunction` in Rust (`TransferOut` aborts first). Likewise a 1025-message list
whose first entry is restricted: `TooManyUpwardMessages` in the model,
`RestrictedHostFunction` in Rust.

Accept/reject is identical in every case — only the variant stored in `parachain_log` differs.
Emission order is the more useful diagnostic (it names the call the PVF actually got wrong) and
is the only order a streaming dispatcher can produce without buffering, so the model should
probably follow Rust here.

**Spec feedback**: §4.1/§4.3 should state whether the failure reason is the first violation in
emission order or the highest-priority violation in the whole list.

## M-5: `RefineOutputTooLarge` threshold — resolved

Quint `4cff218575` introduced the separate 40 KiB encoded upward-message budget. Rust enforces
that streaming budget in `send_upward_message`, then retains the actual Gray Paper 48 KiB
combined-output check as a backstop.

## M-6: `UpgradeService` ignored the declared `len` — resolved

`quint/accumulate.qnt:315` gates the service self-upgrade on the request status of the
`(hash, len)` pair (`preimageAvailable`: `Provided` or `Rerequested`). Rust used JAM's
`lookup`, which is keyed by hash alone and still finds a forgotten preimage until it is
expunged, so it upgraded on a wrong `len` and on forgotten code. It now asks JAM's
`query(hash, len)` and accepts the same two states (`accumulate_upgrades.rs`,
`service_upgrade_*`).

## M-7: `is_valid_val_count` is dead code

`service/src/constants.rs` defines `is_valid_val_count` (multiples of 3 in `[6, 3 * CORE_COUNT]`,
the model's `ValCount`). Nothing calls it: the §5.3 length check is JAM's own. The vendored
host's `designate` takes a bounded set and rejects a length outside `valcount`, which the
service logs as `DesignateRejected` — the model's rule, so the two sides now agree.

Fix: delete `is_valid_val_count`.

## M-8: SCALE discriminants differ between Rust, the design doc and the model

Each enum's variant order is its wire ABI. Against the design doc's listing:

- `RefineLog`: Rust has `InvalidAuthorizerQueue` at 7 and `MalformedPayload` at 8
  (`service/src/work_digest.rs`); the design doc has them the other way round. The model has no
  `MalformedPayload` at all.
- `UpwardMessage`: Rust follows the design doc; the model (`quint/messages.qnt`) puts
  `UpgradeService` last instead of before the four Coretime-only calls.
- `AccumulateLog`: Rust follows the design doc for all 17 variants and appends an 18th,
  `InvalidCodeHash`, that nothing emits any more (the model dropped `InvalidCodeHashAcc`).
- `InsufficientBalanceReason`: Rust extends the design doc's two variants with
  `StagedValidatorKeys`, `IncomingTransfer` and `ParaInfo`, produced only by the §6.1 write
  backstop.

No behavioural consequence today: the digest is produced by this service's Refine and consumed
by its own Accumulate, and every discriminant is 1 B, so no size computation moves. It matters
the moment anything outside this repo decodes a work digest or a log.

Fix: swap `MalformedPayload` and `InvalidAuthorizerQueue` in Rust, drop the dead
`InvalidCodeHash`, and align `quint/messages.qnt` with the design doc.

## M-9: `AssignCore`'s empty-queue documentation — resolved

The doc no longer promises that an empty `queue` cancels a cached entry. Both sides treat any
malformed queue (empty, over-long, or a short handoff) as a defensive no-op in Accumulate,
since Refine rejects them first (Quint `6b8f7292e0`).

## M-10: no model invariant is checked against Rust

The replay harness ([QUINT_REPLAY.md](./QUINT_REPLAY.md)) replays 73 deterministic fixtures and
streaming fuzz campaigns, comparing storage, logs, head commitments and JAM effects after every
transition. It covers Accumulate only: Refine divergences (M-4) are still found by reading.

None of the 32 invariants in `quint/invariants.qnt` is asserted on the Rust side. Several are
cheap to port against real storage and would catch derived-constant drift of the kind that
made Asset Hub's baseline under-reserve its pending-assign queues:
`used_balance_consistency`, `pending_authorizer_cores_consistent`,
`pending_authorizer_apply_at_future`, `parachain_log_within_capacity`.

## M-11: the model decides the §6.5 supervised-service outcomes; Rust can only refuse

**Neither is wrong; the host is behind both.**

Since spec `124a7362235` the model carries supervised services as real state
(`quint/foreign_services.qnt`, the `foreignServices` var) and decides five of §6.5's
six operations: a `Service`-targeted `forget` runs the same two-step expunge as §6.1
and emits `ForgetAgainAt`; a `Service`-targeted `solicit` creates the request, rescues
an `Unrequested` one, or fails `AlreadySolicited`; `remove_service_storage` shrinks the
store idempotently; `eject_service` refuses `TargetIsSelf`/`NotEmpty`/`CreatedThisSlot`
and otherwise deletes; `set_service_supervisor` moves the link. Since `148fbfb7856` it also
tracks JAM balances as ghost state: a `solicit` the target cannot keep paying for fails
`TargetCannotAfford`, and `eject_service` pays both of the target's balances into this
service's regular balance.

`service/src/accumulate/foreign_services.rs` reaches none of those verdicts. The
vendored PolkaJAM host has no supervisor relation at all, so the Parachain Service is
never any service's effective supervisor and all five refuse with `UnknownService` (when
JAM does not know the target) or `NotSupervised`. Only `create_service` agrees with the
model: both fund the new service with exactly its threshold balance, check funds before a
`desired_id`, and report `CannotAfford` and `IdTaken` alike. The exception is
`new_supervisor_balance`, which the host cannot honour: Rust reports `CannotAfford` where
the model creates the service.

So for any trace that exercises §6.5, the model's `foreignServices` and log diverge
from Rust's log on every frame. This is a **host** gap, not a spec or implementation
error on either side — see [DECISIONS.md](./DECISIONS.md) D-13 for the per-operation
table and the two residual `create_service` gaps.

Consequence for the replay harness: it compares neither `foreignServices` nor the model's
ghost balances, and a §6.5-carrying frame must be checked for the *refusal* log rather
than the model's outcome.

**Spec feedback**: none for §6.5's semantics, which are self-consistent. The gap is
JAM's.

## M-12: incoming transfers cannot expose a supervisor-balance selector yet

Quint `6c74e58525` adds `to_supervisor_balance` to each queued incoming transfer;
`0cfb689cad` adds the corresponding model field, and since `148fbfb7856` the model credits
each transfer to the balance it names. Rust stores and encodes that
flag and reserves its extra byte (196 balance units per worst-case bucket).
The vendored JAM `TransferRecord` has no destination-balance selector and its
host only delivers transfers to the regular balance. Rust therefore records
`false`. Forward the actual selector when the host exposes supervisor transfers;
the storage format already supports both values. This is a host limitation,
like the supervised-service operations in M-11, not a normalization of `true`
model transfers to `false`.

## M-13: a future-slot `AssignCore` for a handed-away core — resolved

Since Quint `64713864ffa` the model, like the service, learns of a handoff only from `assign`: a
future-slot `AssignCore` is cached whatever the core's assigner, and the flush that calls `assign`
once it falls due drops the entry and logs `CoreNotAssignable` in the Coretime chain's log. Pinned
by the `future_assign_after_handoff_works` replay fixture.

## M-14: an `assign` JAM rejects for a bad core index — resolved

Since Quint `73d4d9eb9d8` Refine rejects an `AssignCore` naming a core at or above
`C_corecount` with `InvalidCoreIndex`, and Accumulate ignores one defensively. Rust bounds
`core` by the chain's `core_count()`, inactive cores included, so JAM's `assign` can no longer
answer `CORE`, and the dirty-core index's 341-entry bound holds. `WHO` was already unreachable,
so an `assign` now fails only for a handoff. The model fixes `CoreCount` at the full chain's
341.
