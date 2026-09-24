# Model–Implementation Divergences

Places where the Rust implementation and the
[Quint spec](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/) disagree on
observable behaviour or on a derived constant. Found by reading both sides in full against
spec pin `931846282d`.

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
Four of the entries below invert that — the model is wrong and the finding goes upstream.

---

## M-2: `TransferOut` logs failures the model treats as silent no-ops

**Rust is arguably right; the divergence is unpinned and undocumented either way.**

`quint/accumulate.qnt:243-256` appends `TransferFailed` for exactly one shape — a plain move
(`deferred: None`) to a destination that is not this service's supervisor. Every other refusal
falls through as a no-op with no log: a named `source`, either supervisor selector, and the
self-move cases. `service/src/accumulate/transfers.rs:143-167` logs `TransferFailed` on every
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

`quint/refine.qnt:209-231` scans the *finished* upward-message list in a fixed priority:
message count → `set_validator_keys` chunks → assign queues → parachain restrictions → output
size → head declarations. Rust has no such scan — the checks live in the host-call dispatcher
and abort at the first offending call in **emission order**
(`service/src/pvf/executor.rs:198-234` and `:287-292`).

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

`quint/accumulate.qnt:315` gates the service self-upgrade on
`preimageAvailable(payload.codeHash, payload.len)` — the `(hash, len)` pair, matching how the
preimage registry is keyed. `service/src/accumulate/upward.rs:112` destructures `len: _` and
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

## M-8: `UpgradeService` sits at a different SCALE discriminant

`UpwardMessage::UpgradeService` is variant 13 in Rust
(`service-interface/src/upward_message.rs`, ordered per the design doc's §3.3 listing) and
variant 17 in the model (`quint/messages.qnt:266`, ordered with the privileged calls last).
The §6.5 additions narrowed this: both sides now agree on discriminants 0..12
(`RequestCodeUpgrade` .. `ConsumeTransfersUpTo`) and differ only in the tail, where Rust
places `UpgradeService` before the four Coretime-only calls and the model places it after.
`RefineLog`'s ordering differs too — Rust interleaves four implementation-only variants
(`InvalidCode`, `ValidationFailed`, `MalformedPayload`, `HeadDataTooLarge`) among the model's
eight.

No behavioural consequence today: the digest is produced by this service's Refine and consumed
by its own Accumulate, and every variant is 1 B either way, so `refineLogSize` and
`upwardMessageSize` are unaffected. It matters the moment anything outside this repo decodes a
work digest.

Fix: pick one ordering — the design doc's — and align `quint/messages.qnt` to it.

## M-9: `AssignCore`'s empty-queue documentation — resolved

The doc no longer promises that an empty `queue` cancels a cached entry. Both sides treat any
malformed queue (empty, over-long, or a short handoff) as a defensive no-op in Accumulate,
since Refine rejects them first (Quint `6b8f7292e0`).

## M-13: a future-slot `AssignCore` for a handed-away core is rejected only by the model

Quint `6b8f7292e0` tracks each core's assigner as ghost state (`jamCoreAssigners`) and logs
`CoreNotAssignable` as soon as an `AssignCore` names a core this service handed away, whatever
its `jam_slot`. The service cannot read JAM's assigner: it learns of a handoff only when
`assign` fails. It therefore logs `CoreNotAssignable` for a due assign, but caches a
future-slot one, which JAM then rejects at the flush, leaving the entry in place. Streaming
fuzz campaigns that hand a core away and later schedule it for a future slot hit this. The
spec is expected to change here.

## M-10: nothing checks equivalence, and the replay ledger has gone stale

The harness described in [QUINT_REPLAY.md](./QUINT_REPLAY.md) is still a Phase-0 spike.
`service/bin/tests/quint_replay.rs` loads one fixture holding **2 states**, replays one block,
and compares four fields (`head_data`, `total_state_balance`, a recomputed `used_state_balance`,
and log-emptiness) for one para. There is no frame classifier, no ITF codex, and no
implementation of the divergence ledger. Every entry M-2..M-9 above was found by reading, not by
a failing test — which is the reason to expect more.

Separately, none of the 29 invariants in `quint/invariants.qnt` are asserted on the Rust side.
Several are cheap to port against real storage and would catch derived-constant drift of the
kind that made Asset Hub's baseline under-reserve its pending-assign queues:
`used_balance_consistency`, `pending_authorizer_cores_consistent`,
`pending_authorizer_apply_at_future`, `parachain_log_within_capacity`.

The ledger itself now mis-describes the tree. Three of its five entries are stale:

| Ledger entry | Status |
|---|---|
| D-1 — balance encoding width | resolved upstream (spec `459985739f`); the file says so, but its Asset Hub row still carries the pre-`AUTHORIZER_QUEUE_LEN` figure |
| D-3 — chain counter absent from the model | stale: `quint/state_balance.qnt` charges `+ 4 (count)` |
| D-4 — admission threshold 204 vs 196 | stale: both sides compute `IncomingTransferEntryFootprint = 196` |
| D-2 — always-accumulate on non-block steps | still accurate |
| D-5 — headroom slack at mid-trace registration | still accurate, and shrinks to 0 for non-Asset-Hub paras once D-1's shift is retired |

Fix: retire D-1/D-3/D-4 from the ledger, regenerate `minimal_replay.itf.json` under the current
pin so D-1's normalization has nothing left to compensate for, and grow the harness past one
fixture.

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
