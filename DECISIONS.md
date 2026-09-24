# Implementation Decisions

Decisions made while implementing the Proof-of-Concept of the
[Parachain Service design](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/parachain-service-on-jam.md),
in the places where the design under-specifies, contradicts itself, or leaves an
implementation choice open. Each entry names the gap it resolves and what should be fed
back into the spec.

Format: **D-n** — decision, rationale, spec feedback. Entries are deleted once a GitHub
issue exists for them; numbering is not reused.

## D-2: Asset Hub / Coretime ParaIds are compile-time constants

Gap: the design does not specify the required JAM privileges or bootstrap state.

Refine must enforce restricted host functions (§4.3) but is stateless, so it cannot learn
the privileged ParaIds from service storage. The PoC hard-codes
`ASSET_HUB_PARA_ID` and `CORETIME_PARA_ID` in `service-interface`.

Alternative considered: carry a per-item role (Normal/AssetHub/Coretime) in the enforced
authorizer-config prefix (§3.2), which is Coretime-committed via the `assign` flow — no
hardcoding, governable, but a spec change to the prefix shape and a policy obligation on
Coretime to never mark a non-privileged para as privileged.

**Spec feedback**: the design must state where the privileged-para identity lives. Both
options should be listed; constants require a service self-upgrade to migrate Asset Hub or
Coretime to a new ParaId.

## D-4: the AURA authorizer keeps a dev-network sudo bypass

Gap: the AURA authorizer is an example in the design, not an executable protocol.

The authorizer implements the complete §7.1 pipeline — config/token decoding, round-robin
collator-index computation, Merkle-proof verification of the collator key, and a real signature
check (ed25519 `verify_strict` or sr25519, one blob each). One deliberate hole remains: a token
carrying the all-ones `SUDO_KEY` is authorized on any core without a signature, so control
packages can reach a parked core (user decision, 2026-09-02). It must not ship.

**Spec feedback**: none for the bypass, which is PoC-only. The §7.1 canonical proof/encoding
rules still need specifying.

## D-5: the parent-head check's `hash(head_data)` is blake2b-256

New finding (not previously tracked). The design's parent-head check (§5.1 step 3) compares
`parent_head_hash` against `hash(ParaInfo.head_data)` but never names the hash function; the
PVF (via `set_parent_head_hash`) and the service must agree on it. We pin **blake2b-256**,
matching JAM's own preimage hashing (`jam_std_common::hash_raw`).

Scope narrowed by spec `09f868d216b`: `head_data` is now digested under **two** functions.
This entry covers only the §5.1 parent-head check; the head hash a §5.5 commitment *leaf*
carries is **keccak-256**, which the spec now states outright (see D-12). The two are
deliberately separate domains, so a commitment leaf never verifies against a candidate's
declared parent head. Both live in `service/src/hashing.rs`.

**Spec feedback**: §5.1/§4.3 still do not name the function for the parent-head check;
only §5.5's side was pinned upstream.

## D-8: incoming transfers are recorded in one bucket write per block

Gap: transfer gas, admission, and financial reconciliation are incomplete in the design.

The Quint model records each incoming transfer individually. Since one accumulate
invocation fills one bucket (§5.1: a bucket is closed when the invocation that opened it
ends), a literal port re-reads and re-writes the growing bucket per transfer — measured at
551M gas for 1024 same-slot transfers under the polkavm 0.30 gas model, 55x the Gray Paper's
whole per-report budget `Ga = 10M` (`accumulate_gas.rs::incoming_transfer_bench_works`). The
PoC batches: admission is checked per transfer in operand order (identical semantics,
including the count-based cap), the invocation's buckets are filled in memory, and each is
written exactly once. The batched cost measures 8.65M gas under polkavm 0.36 (1.63M under 0.30).

Since spec `17a10dcb3f`, `MAX_TRANSFERS_PER_BUCKET` (512) can also split one invocation
across several buckets; the batching writes one entry per bucket actually used, so the
property is unchanged.

**Spec feedback**: the model's per-transfer processing is fine as a spec of *meaning*, but
§5.1 should note that recording must be batched per invocation — a conforming literal
implementation cannot fit its own gas budget.

## D-10: upward messages use one SCALE-encoded host call

Resolved by Quint `4cff218575`: the child PVF passes every [`UpwardMessage`](service-interface/src/upward_message.rs)
through `send_upward_message(msg)` as one SCALE blob. This removes per-variant register ABIs,
including the earlier special case for `transfer_out`, and makes the parachain-visible encoding
the sole message definition.

## D-11: only the deferred transfer mode is executable on the vendored host

Gap: per-parachain quotas are not backed by the real service balance, and transfer gas
and admission are incomplete.

§5.1's `transfer_out` presupposes a Gray Paper >= 0.8 `transfer`: a `source` selector, a
regular/supervisor balance pair on each side, and a plain-move mode that runs no destination
code. The vendored PolkaJAM host is GP 0.7.2 — its `transfer(dest, amount, gas_limit, memo)`
always defers, always debits this service, and knows one balance per service; "supervisor"
does not appear anywhere in `jam-types`.

The PoC therefore accepts the full spec shape on the wire but executes only what the host
can express, refusing the rest with the error the design itself assigns:

| Requested shape | Outcome |
|---|---|
| `source = Some(_)` | `UnknownSource` / `SourceNotSupervised` |
| either supervisor selector set | `DestinationNotSupervised` |
| `deferred = None` (plain move) | `DestinationNotSupervised` |
| `deferred = Some((memo, gas))` | forwarded to JAM `transfer` |

This is not merely a degradation: the Quint model reaches the same verdicts, because a plain
move requires supervision of `dest` that the Parachain Service never holds, and a foreign
`source` always fails. The gap is confined to the self-move cases the model leaves abstract.

**Spec feedback**: §5.1 should say which `transfer_out` shapes are expected to be reachable
in practice. If only the deferred mode ever is, the selectors and `source` are dead wire
fields for the foreseeable future and should be documented as forward-compatibility only.

## D-12: the §5.5 commitment tree pairs adjacent leaves and promotes odd elements

New finding. §5.5 fixes the leaf contents, the leaf ordering, and the element hash, but
says nothing about **how leaves are paired** or **what happens to a trailing odd element at
a level**. Both choices change the root, so no two implementations interoperate until one is
written down — the Quint README flags this as an open question against the design.

The PoC pins the Quint model's reading: hash adjacent pairs left to right, and promote a
trailing odd element to the next level **unchanged** rather than duplicating it. This matches
the `binary-merkle-tree` crate, which is what Polkadot already uses for its own Merkle roots
and is already a workspace dependency, so a verifier can reuse the familiar construction.

~~The PoC also has to pin one thing §5.5 leaves implicit: the leaf's `head_hash` is
`blake2b-256` of the head data, per D-5.~~ **Resolved upstream** in spec `09f868d216b`:
§5.5 now states that a leaf's `head_hash` is `keccak_256`, so the leaf and the tree elements
use the same function and only the §5.1 parent-head check remains blake2b-256 (D-5).

**Spec feedback**: §5.5 must still state the pairing rule and the odd-element rule. Until
then any independent implementation is likely to compute a different root.

---

# Implementation Findings

Spec issues surfaced while implementing the PoC that did not need a user decision but must
flow back into the design doc / Quint model. Numbered **F-n**; `TODO`/`FIXME` markers in the
code reference the same issues.

## D-13: only `create_service` of §6.5 is executable on the vendored host

Gap: §6.5 presupposes JAM supervision the vendored host does not have.

§6.5 adds six operations a supervisor may perform on a supervised service:
`solicit` and `forget` with a `Service` target, `remove_service_storage`,
`eject_service`, `set_service_supervisor` and `create_service`. All six turn on a
per-service **supervisor** link that the supervisor may act through — a Gray Paper
>= 0.8 concept. The vendored PolkaJAM host is GP 0.7.2 and has none of it:

| §6.5 operation | Vendored GP 0.7.2 host |
|---|---|
| `solicit(Service(s), …)` | only `solicit(hash, len)` for the caller's own store |
| `forget(Service(s), …)` | only `forget(hash, len)` for the caller's own store |
| `remove_service_storage` | only `get_foreign_storage` — reads, never writes |
| `eject_service` | `eject(target, code_hash)` needs `target` to have called `zombify` naming us as ejector, which a supervisor cannot do on its behalf |
| `set_service_supervisor` | absent; `bless` sets the four global privileges, not a per-service link |
| `create_service` | `create_service(…, new_service_id)` — fully supported, `desired_id` included |

`ServiceInfo::parent_service` does record a creator, but no host call is gated on
it and nothing can assert or transfer it, so it is informational only. The
Parachain Service is therefore never any service's *effective supervisor*, and the
five supervision-gated operations accept the full spec shape on the wire and
refuse with the error the design itself assigns — `UnknownService` when JAM does
not know the target, else `NotSupervised`. This is the same reasoning D-11 applies
to `transfer_out`'s unsupported shapes, and it keeps the refusals honest rather
than silently dropping a parachain's request.

`create_service` really runs. Two residual gaps:

- **The balance selectors are inexpressible.** GP 0.7.2 knows one balance per
  service, so `source_supervisor_balance` / `new_supervisor_balance` cannot be
  honoured. `ServiceCreationResult` has no variant for a refused-because-
  unrepresentable request, so the PoC reports `CannotAfford` — the same missing-
  error-code gap as F-14.
- **A `desired_id` outside the protected range is silently allocated elsewhere.**
  The host honours `new_service_id` only while the caller holds `registrar` *and*
  the index is below `NEW_ID_BASE` (2^16, matching the model's
  `MinPublicServiceIndex`); otherwise it ignores the request and allocates. §6.5
  defines `IdTaken` for an index in use but says nothing about one out of range,
  so such a call reports `Created(other_id)`.

**Spec feedback**: §6.5 should state which of these operations are expected to be
reachable before JAM ships GP >= 0.8 supervision, and `ServiceCreationResult`
needs a variant for a `desired_id` outside the protected range.

## F-1: §7.1 needs the anchor timeslot, but the refinement context does not carry it

The AURA authorizer computes `collator_index` from "the anchor timeslot read from the
refinement context" (§7.1 step 3). The Gray Paper's `RefineContext` exposes only the
**lookup-anchor** slot; the anchor's own slot is not available in-core. The PoC uses
`lookup_anchor_slot` behind a FIXME, which would let a collator pick any lookup anchor
mapping to its own index. The design needs either a JAM change (anchor slot in the context)
or a different slot source. Extends the AURA-authorizer gap.

## F-8: model fidelity nits found while porting

- `validator_keys.qnt`'s module doc claims partial appends are balance-charged; the code
  (correctly, per §6.1's Asset Hub baseline pre-provisioning) never charges. Doc drift.
- `state.qnt`'s `kvEntryFootprint` comment says map tag `0x07`; the §3.1 table says
  `key_value_storage = 0x08`.

## F-9: AURA config gains a target-service field

To close the AURA authorizer's "does not require work items to target the Parachain Service", the
PoC adds `parachain_service: ServiceId` to the AURA `AuthorizerConfig` and rejects packages
whose items target any other service. §7.1's config struct needs the field.

## F-10: the 1024-message digest cap has no gas headroom against `Ga`

A reachable worst-case digest (1024 `solicit`s, 36 KiB — fits the report bound) replays at
61.7M gas under the polkavm 0.36 interpreter, 6.2x the Gray Paper's per-report accumulate
budget `Ga = 10M` (`accumulate_gas.rs`). Since spec `ed73e50f0c` the gas gate skips such a
report unless it declares that much gas (F-17), so a candidate Refine already validated is
never enacted. §4.3's `MAX_UPWARD_MESSAGES_PER_DIGEST` needs deriving from `Ga` (with
margin), not picked independently. Pinned by `solicit_bench_works` / `set_kv_bench_works`.

## F-11: several message types cannot reach the 1024-message cap anyway

The digest is part of the work-report's elective data, capped by Gray Paper
`Wr = 48 KiB`. A `TransferOut` encodes to ~134 B (memo alone is 128 B), so at most ~345 fit
in a report; 1024 of them encode to 137 KiB. The §4.3 cap should be stated per encoded
size (i.e. derived from `Wr`), not as a flat message count.

## F-12: the non-candidate gas budgets need sizing

The design expects the service to be always-accumulate (§2) but never sizes the
`always_acc` privileges allotment that pays for operand-less maintenance work, and since spec
`ed73e50f0c` §5.1 requires the always-accumulate phases to stay within it: bounded and
benchmarked. Nothing bounds them yet. Measured worst case: flushing a due `assign` for every
core (341 entries, full 80-hash queues) in one block costs 58.6M gas, ~172k per assign
(`accumulate_gas.rs::due_assign_bench_works`, polkavm 0.36 interpreter); steady state is one
core per slot.

Similarly, incoming-transfer recording is paid by the transfers' own `gas_limit` (the JAM
scheduler adds it to the invocation unconditionally), so the service's `min_memo_gas` must
cover the measured ~8.5k-per-transfer recording cost with margin — a token value like the
mock's 100 would be under water.

## F-13: forwarded transfer gas multiplies past `Ga` despite both caps

Sharpens F-10. `Ω_T` charges each replayed `TransferOut`'s forwarded gas to the sender's
meter. Both bounds are individually enforced — per-transfer `MAX_TRANSFER_GAS = Ga/100`
(#17), per-report ~345 transfers under `Wr` (F-11) — but their product exceeds `Ga`: an
Asset Hub digest of 331 transfers to a destination demanding `min_memo_gas =
MAX_TRANSFER_GAS` measures 38.1M gas — 3.8x `Ga`, of which 33.1M is forwarded
(`accumulate_gas.rs::transfer_out_max_gas_bench_works`). Destinations are user-chosen, so this
is reachable. The replay loop needs a **cumulative** forwarded-gas budget per digest, not only
a per-transfer cap — which spec `ed73e50f0c`'s gas gate now provides: it charges each deferred
transfer's gas against the limit the report declared and skips a report that cannot cover it
(F-17), instead of letting it run dry after ~90 replays.
Only Asset Hub digests may carry `TransferOut`, and 345 is the most ~134-B transfers that
fit the `Wr = 48 KiB` report bound.

NOTE: §5.1's reworked `TransferOut` changes the arithmetic above. A deferred transfer now
encodes to ~146 B (the memo still dominates) and a plain move to ~15 B, so the per-report
count is no longer ~345 for every shape. The forwarded-gas conclusion is unchanged — only
deferred transfers carry gas — but the bound must be re-derived per shape.

## F-14: no error code exists for a sender-side transfer-gas cap

§5.1 moved the transfer gas limit from a replay-time lookup of the destination's
`min_memo_gas` to a caller-supplied value, and defines six `TransferError` variants. None of
them covers the PoC's own protective refusal when the requested gas exceeds
`MAX_TRANSFER_GAS` (D-6, F-13) — the cap that stops one digest's transfers from burning the
whole accumulate budget. The PoC reports `InsufficientServiceBalance`, which is the closest
available variant but describes balance rather than gas.

Either `TransferError` needs a variant for a refused gas request, or §5.1 must state that the
service may not impose such a cap and instead relies on a per-digest cumulative budget
(F-13). The two findings should be resolved together. Spec `ed73e50f0c`'s gas gate already
charges each deferred transfer's gas against the limit its report declared (F-17), which is
that cumulative budget; the per-transfer cap is now redundant.

## F-17: the §5.1 gas gate runs on provisional costs

Spec `ed73e50f0c` skips a report whose base cost plus content-derived cost exceeds the gas the
report declared, so no report is started that cannot be paid for in full. The PoC implements
the gate with two unbenchmarked constants (`service/src/constants.rs`): `REPORT_BASE_GAS` = 5M,
sized for a full 64 KiB log rewritten under a 4 KiB head, and a flat `UPWARD_MESSAGE_GAS` =
250k per message, covering the fixed-size variants; each deferred `TransferOut`'s forwarded gas
is added on top. Messages whose cost scales with their payload or with stored state are not
covered: a final `SetValidatorKeys` chunk onto a nearly full staging buffer measured ~14.2M
gas, more than the whole `Ga = 10M`. Such a report clears the gate and can still run out of
gas mid-replay — the case the gate exists to prevent.

**Spec feedback**: the costs must come from per-variant benchmarks, with formulas for the
variants that scale with the state they touch (`SetValidatorKeys`, `SetKV`,
`CleanUpBucketsUpTo`).

## F-16: the model's abstract hashes discard their domain, blocking trace replay

Found while designing the Quint trace-replay harness ([QUINT_REPLAY.md](./QUINT_REPLAY.md)).

Three constructors map into `Hash = { hashBytes: int }` untagged, so their images overlap:
`headHash(1)`, `{ vchBytes: 1 }.asPreimage()` and `listHash(List(1))` all yield `{ hashBytes: 1 }`
(`types.qnt:191/59/195`; disproof in `upstream-feedback/f-16-hash-injectivity.qnt`).

This is not a model defect. Every `Hash`-consuming site is fed by exactly one constructor —
`accumulate.qnt:357` compares `headHash` against `headHash` — so the model's comparisons are
injective and sound.

It does block replay, which runs the other way: `{ hashBytes: 1 }` back to 32 real bytes, where
the bytes depend on a domain the trace no longer records. In `paraRegisterOperateCleanupTest`
frame 3, `{ hashBytes: 1 }` is both a `preimageRegistry` key (a 64 KiB validation code) and a
digest's `parentHeadHash` (an 8-byte head). The PoC harness recovers the domain from the ITF
field path, which works only because each consuming site is single-domain.

**Spec feedback**: domain-tag the hash constructors, as `merkleHash` (`head_commitment.qnt:25-28`)
already does for leaves vs nodes, so traces round-trip without an out-of-spec lookup table.

## Retired decisions

Entries are deleted once a GitHub issue exists for them (see line 9). The identifiers below were
retired from this file but are still cited by code/docs; each maps to its GitHub issue if one exists,
or "issue missing" if none could be found (searched the whole non-vendor tree and all vendored
comments; no `github.com/.../issues/NNN` URL occurs near any of them). This table keeps every `[DF]-N`
citation in the repo resolvable — to a live `## D-n:`/`## F-n:` heading above or to a row here.

| Identifier | Retired as (context) | Issue |
|---|---|---|
| D-1 | child-PVF ABI is host-call based for both results and input; the PoV arrives as work-item extrinsic 0 via `fetch` (spec §3.2, §4.2) | issue missing |
| D-3 | state-balance accounting uses the exact §6.1 formulas, with `Balance = u64` | issue missing |
| D-6 | `TransferOut` replay looks up the destination's `min_memo_gas`, capped by `MAX_TRANSFER_GAS` (F-13) | issue missing |
| D-7 | `assign_core` queues shorter than 80 are cycle-repeated | issue missing |
| D-9 | transfer-gas cap — orphan: never a DECISIONS.md heading in this repo's history; folded into D-6 (`service/src/constants.rs:43` cites "D-6/D-9") | issue missing |
| F-4 | the queued-transfer count needs a counter in state | issue missing |
| F-5 | §6.1 sizing tables need re-deriving for the real wire types | issue missing |
| F-6 | `UpgradeService` verifies actual preimage availability, not registry membership | issue missing |
| F-2 | §4.3's `import_segments() -> Vec<SegmentMeta>` has no host-call backing — **accepted upstream**: spec `931846282d` drops it from the §4.3 table | [#11883](https://github.com/paritytech/polkadot-sdk/pull/11883) |
| F-3 | no error codes for oversized `set_head` / oversized `assign_core` queues (retired; cited nowhere today) | issue missing |
| F-7 | no log events for failed JAM `assign` / `designate` host calls (retired; cited nowhere today) | issue missing |
| F-15 | a failed `assign` left no log, unlike `designate` — **accepted upstream**: spec `6b8f7292e0` adds `CoreNotAssignable` | [#11883](https://github.com/paritytech/polkadot-sdk/pull/11883) |

Notes: F-2, F-3, and F-7 were retired in the same pass and are cited nowhere in the tree (included as
rows so every `[DF]-N` in the repo still resolves). The nearest upstream artifact is the design PR
[#11883](https://github.com/paritytech/polkadot-sdk/pull/11883), which is not a per-entry issue. F-2 has
since been resolved there outright: spec `931846282d` removed `import_segments()` from §4.3, so the
host call is gone from `HostCall` too.
