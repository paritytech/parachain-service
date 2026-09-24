# Model–Implementation Divergences

Places where the Rust implementation and the
[Quint spec](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/) disagree on
observable behaviour or on a derived constant. Found by reading both sides and by trace replay;
checked against spec pin `64713864ffa`.

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

## M-2: `TransferOut` logs failures the model treats as silent no-ops

**Rust is arguably right; the divergence is unpinned and undocumented either way.**

`quint/accumulate.qnt:242-255` appends `TransferFailed` for exactly one shape — a plain move
(`deferred: None`) to a destination that is not this service's supervisor. Every other refusal
falls through as a no-op with no log: a named `source`, either supervisor selector, and the
self-move cases. `service/src/accumulate/transfers.rs:147-189` logs `TransferFailed` on every
refusal path.

The model pins its own reading:
`quint/tests/transfers_test.qnt:76` (`transferOutPlainMoveNeedsSupervisionTest`) walks six
`TransferOut` shapes and asserts the log holds exactly `[{id: 77}, {id: 82}]`. Replaying that
same sequence through Rust also logs ids 79, 80 and 81. So this is a `parachain_log` state
divergence, and it is the first thing a trace replay of that test will hit.

[DECISIONS.md](./DECISIONS.md) D-11 tabulates the same refusals and claims "the Quint model
reaches the same verdicts". That holds for the accept/reject decision and not for the log.

Compounding it: these paths have no Rust test at all. `service/bin/tests/common/mod.rs:40`
hard-codes `source: None` and both selectors `false`, so `UnknownSource`,
`SourceNotSupervised` and the selector refusal never execute in the suite.

Fix: decide whether a refused transfer is observable, then make one side match — and add the
missing Rust cases regardless.

**Spec feedback**: §5.1 should state which `transfer_out` refusals are logged. Silent failure
of a balance move is a poor default; the model's own `id` echo-back exists precisely so the
parachain can reconcile.

## M-3: the model drops `ForgetAgainAt` on the code-upgrade paths — resolved

The two-phase §5.2 lifecycle has no reap and no activation release: `Apply`, a superseding
`Announcement` and, since Quint `71e422dc6d`, `ParachainSetValidationCode` all leave the
displaced code referenced until the parachain forgets it. No code-upgrade path calls
`removeReferencer` any more, so there is no `ForgetAgainAt` left to drop.

## M-4: Refine reports a different `RefineLog` variant for the same PVF

**Neither is wrong; the two orderings need reconciling.**

`quint/refine.qnt:233-264` scans the *finished* upward-message list in a fixed priority:
message count → `set_validator_keys` repetition and chunk size → assign queues → parachain
restrictions → 40 KiB message budget → output size → head declarations. Rust has no such scan —
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

## M-6: `UpgradeService` ignores the declared `len`

**Rust is weaker; low severity.**

`quint/accumulate.qnt:326` gates the service self-upgrade on
`preimageAvailable(payload.codeHash, payload.len)` — the `(hash, len)` pair, matching how the
preimage registry is keyed. `service/src/accumulate/upward.rs:114` destructures `len: _` and
calls `is_available(&code_hash)`, which takes no length at all
(`vendor/polkajam/crates/jam-pvm-common/src/host_calls.rs:532`).

So an `UpgradeService` declaring a wrong `len` upgrades in Rust and is rejected with
`ServiceUpgradePreimageMissing` by the model. Severity is low — a hash pins its own preimage,
so the length is determined and cannot select a different blob — but the field is carried on
the wire and then not validated, which is worse than not carrying it.

Fix: either check `len` against the looked-up blob, or drop it from the message.

**Spec feedback**: §5.4 should say whether `len` is authoritative or advisory.

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

The replay harness ([QUINT_REPLAY.md](./QUINT_REPLAY.md)) replays 71 deterministic fixtures and
streaming fuzz campaigns, comparing storage, logs, head commitments and JAM effects after every
transition. It covers Accumulate only: Refine divergences (M-4) are still found by reading.

None of the 31 invariants in `quint/invariants.qnt` is asserted on the Rust side. Several are
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
and otherwise deletes; `set_service_supervisor` moves the link.

`service/src/accumulate/foreign_services.rs` reaches none of those verdicts. The
vendored PolkaJAM host is Gray Paper 0.7.2 and has no supervisor relation at all, so
the Parachain Service is never any service's effective supervisor and all five refuse
with `UnknownService` (when JAM does not know the target) or `NotSupervised`. Only
`create_service` agrees with the model, including `desired_id` honouring and `IdTaken`.

So for any trace that exercises §6.5, the model's `foreignServices` and log diverge
from Rust's log on every frame. This is a **host** gap, not a spec or implementation
error on either side — see [DECISIONS.md](./DECISIONS.md) D-13 for the per-operation
table and the two residual `create_service` gaps.

Consequence for the replay harness: `foreignServices` joins `prevSvc` and friends in
[QUINT_REPLAY.md](./QUINT_REPLAY.md)'s "not compared" list until the host gains GP >= 0.8
supervision, and a §6.5-carrying frame must be checked for the *refusal* log rather than
the model's outcome.

**Spec feedback**: none for §6.5's semantics, which are self-consistent. The gap is
JAM's.

## M-12: incoming transfers cannot expose a supervisor-balance selector yet

Quint `6c74e58525` adds `to_supervisor_balance` to each queued incoming transfer;
`0cfb689cad` adds the corresponding model field. Rust stores and encodes that
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

## M-14: an `assign` JAM rejects for a bad core index is unspecified

Refine checks only an `AssignCore` queue, never `core`, and the model treats every well-formed
assign as succeeding. JAM rejects a core index ≥ 341 with `CORE`. Rust logs nothing for it: an
inline assign is dropped (any entry cached for the core stays), and a cached one is retried at
every block (`apply_due_assigns`, marked `TODO`). Worse, the dirty-core index holds at most 341
entries, so once more than 341 distinct cores are waiting, the `expect` in
`DirtyCores::upsert` panics and the invocation reverts to its last checkpoint. (JAM's `WHO`
rejection is unreachable: every `u32` service id fits.)

Fix: bound `core` below the core count in Refine (§4.3), and say what an `assign` rejected for
another reason than a handoff does.
