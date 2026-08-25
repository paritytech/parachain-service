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

## M-3: the model drops `ForgetAgainAt` on the code-upgrade paths

**The model is wrong.**

`quint/accumulate.qnt:405-411` returns `logs: List()`, discarding both the timed-out reap's log
and the activation's log, and `quint/code_upgrades.qnt:122-126` returns `log: None`, discarding
the superseded pending code's. All three come from `removeReferencer`, which produces
`ForgetAgainAt` when a first `forget` only unrequests a still-`Provided` preimage.

Rust threads `&mut logs` through all three sites and keeps the entry
(`service/src/accumulate/package.rs:96`, `:106`; `service/src/accumulate/code_upgrades.rs:87`).

Rust is right, and the model is inconsistent with itself: `quint/management.qnt` keeps the same
log for `parachainSetValidationCode` and `parachainCleanUp`. Without the entry the parachain
never learns when the second, expunging `forget` becomes due, so its state balance stays
charged for a preimage it asked to release — with no on-chain trace of why.

**Spec feedback**: §5.2's reap and activation paths must surface `ForgetAgainAt` exactly as
§6.1's release paths do.

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

## M-5: `RefineOutputTooLarge` fires at a different threshold

**Rust is right; the model is conservative.**

`quint/refine.qnt:119-125` charges the worst case on both variable parts — head data at 4 098 B
and the auth trace at 256 B — so its message budget is a fixed ~44.7 KiB.
`service/src/lib.rs:41-54` measures the actual encoded digest plus the actual
`auth_trace().len()` against `MAX_REFINE_OUTPUT_SIZE`.

A candidate with small head data therefore gets roughly 4 KiB more message room in Rust than
the model allows. Rust's is the Gray-Paper-accurate check — `W_R` bounds real octets — so the
model is merely stricter, not unsafe. But the boundary is not the same, and any replay near it
will diverge.

**Spec feedback**: §4.1 step 7 should say the check is against actual encoded size; the model
should measure `headData` rather than charging its cap. This is blocked on the model's
`HeadData` being an opaque `int` with no measurable length (the same abstraction that makes
`upwardMessageSize` charge `ParachainSetHead` its 4 102 B worst case, `quint/refine.qnt:106`).

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

## M-7: `is_valid_val_count` is dead code, and the live check is stricter than the model

`service/src/constants.rs:39` defines `is_valid_val_count` (multiples of 3 in `[6, 3 * CORE_COUNT]`,
the model's `ValCount` at `quint/types.qnt:91`). Nothing calls it. The actual §5.3 length check
is `OpaqueValKeysets::try_from` at `service/src/accumulate/validator_keys.rs:36`, a `FixedVec`
of exactly `val_count()` = 1 023 — equality, not set membership.

The stricter check is correct (JAM's `designate` takes the protocol's exact validator count) and
is already recorded as [DECISIONS.md](./DECISIONS.md) F-8. What is new here is the dead helper:
it encodes the model's wrong rule in live code, one `use` away from being adopted as "the fix".

Fix: delete `is_valid_val_count`.

## M-8: `UpgradeService` sits at a different SCALE discriminant

`UpwardMessage::UpgradeService` is variant 9 in Rust
(`service-interface/src/upward_message.rs:93`, ordered per the design doc's §3.3 listing) and
variant 13 in the model (`quint/messages.qnt:192`, ordered with the privileged calls last).
`RefineLog`'s ordering differs too — Rust interleaves four implementation-only variants
(`InvalidCode`, `ValidationFailed`, `MalformedPayload`, `HeadDataTooLarge`) among the model's
eight.

No behavioural consequence today: the digest is produced by this service's Refine and consumed
by its own Accumulate, and every variant is 1 B either way, so `refineLogSize` and
`upwardMessageSize` are unaffected. It matters the moment anything outside this repo decodes a
work digest.

Fix: pick one ordering — the design doc's — and align `quint/messages.qnt` to it.

## M-9: `AssignCore`'s empty-queue documentation describes behaviour no side implements

`service-interface/src/upward_message.rs:76` documents "An empty `queue` cancels any cached
entry for the core (no JAM call)". Nothing cancels: `service/src/accumulate/assigns.rs:22-24`
returns without touching `pending_assigns` or the dirty-core index, and
`quint/accumulate.qnt:264` does the same, both calling it defensive because Refine already
rejects an empty queue (`quint/refine.qnt:61`).

Unreachable, so this is a comment bug rather than a behaviour bug — but a cancel path is a
plausible thing for someone to *want*, and the doc currently promises it exists.

Fix: delete the sentence, or implement cancellation and give it a Refine-side rule.

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
