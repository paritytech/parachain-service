# Quint Trace Replay — Accumulate Equivalence Testing

> Every format, constant and CLI flag in this document was checked against
> `quint 0.32.0`, the vendored spec at `vendor/polkadot-sdk-quint`
> (`4a22816d`), and the Rust tree. Claims that are **not** yet verified are
> marked **(unverified)** and are resolved by Phase 0.

## Goal

Replay Quint-generated ITF traces through the Rust **Accumulate** pipeline and
assert state equivalence against the model, field by field, modulo an explicit
ledger of known model-vs-implementation divergences.

This is *accumulate* equivalence, not whole-service equivalence. Refine is
bypassed: the model's PVF is an oracle with no PoV bytes, so there is nothing to
feed a real PVF. We take the model's refine **output** (`lastStepWorkResults`)
as the input to Rust's Accumulate. See [Out of Scope](#out-of-scope).

## Direction

Quint is the oracle; Rust must match it. Where they disagree and the *model* is
wrong, the finding goes to `upstream-feedback/` — see
[Upstream feedback](#upstream-feedback-generated-by-this-work).

---

## Trace sources

Two, with different jobs.

### A. `quint test` — deterministic, named scenarios (primary)

`tests.qnt` exists for exactly this. Its header (`tests.qnt:6-12`):

> *"the tests below need DETERMINISTIC scenarios — input → output relationships
> the Rust harness can replay."*

```sh
quint test --out-itf='out_{test}_{seq}.itf.json' --seed=1 tests.qnt
```

All 61 runs pass and emit one ITF file each. **But most are not replayable as
traces** — they are pure-value assertions wrapped in `init.then(expectThat(…))`,
so they emit only the init frame. Measured:

| | count |
|---|---|
| runs emitting ≥1 **block** transition | **9** |
| runs emitting ≥1 state change | 10 |
| runs emitting no transition at all | 51 |

The nine, by block count:

| test | blocks | states |
|---|---|---|
| `transferOutPlainMoveNeedsSupervisionTest` | 6 | 13 |
| `codeUpgradeLifecycleTest` | 5 | 11 |
| `paraRegisterOperateCleanupTest` | 3 | 7 |
| `staleParentSetHeadIgnoredTest` | 2 | 5 |
| `staleParentCandidateRejectedTest` | 2 | 5 |
| `solicitRejectedAtBalanceCapTest` | 2 | 5 |
| `headCommitmentTracksChangedHeadsTest` | 1 | 5 |
| `setHeadNonExistentParaNoOpTest` | 1 | 3 |
| `multipleWorkPackagesAccumulateTest` | 1 | 3 |

(`chainMustNotSkipBucketTest` has 2 state changes but no block — same-slot
transfer applications.)

So the deterministic asset is ~23 block transitions, not 61 scenarios. That is
still the right foundation, and growing it is a spec change we should propose
upstream rather than work around locally.

### B. `quint run --mbt` — randomized breadth (secondary)

```sh
quint run --out-itf=fuzz.itf.json --seed=<n> --max-steps=20 --mbt main.qnt
```

`--mbt` adds two per-frame fields — `mbt::actionTaken` (the action name) and
`mbt::nondetPicks` (every sampled input). Without it the trace records states
only, and the harness cannot tell which of the three step kinds fired. It is
**`quint run`-only**; `quint test` has no `--mbt`.

Realism note: with `--seed` set, `--max-samples` defaults to **1** — there is no
search, just one arbitrary walk. Seed 42 at `--max-steps=10` yields:

```
init, provisionPreimage, stepRefineAccumulate, stepIncomingTransfer,
stepIncomingTransfer, stepIncomingTransfer, stepRefineAccumulate,
stepIncomingTransfer, stepRefineAccumulate, stepRefineAccumulate,
stepIncomingTransfer
```

No code-upgrade lifecycle, no log pruning. **A random walk cannot be promised to
cover named scenarios.** Fuzz traces are for finding surprises; source A is for
coverage.

### Determinism and committing

The **states** are seed-deterministic — verified across repeated runs, with
`--n-threads=1`, and on both the `rust` and `typescript` backends. The **file**
is not: `#meta` embeds a wall-clock `timestamp` and a dated `description`.

Therefore: strip `#meta` before committing, and commit the normalized JSON.

Do **not** gate CI on "regenerate and diff". `vendor/polkadot-sdk-quint` is a
submodule that has already been force-push re-pinned once (VENDOR.md), so that
gate turns upstream spec churn into red CI here. Regenerate deliberately via
`just quint trace-generate`, review the diff, commit.

---

## The real ITF format

Verified output, not reconstructed from docs. ITF is the **Informal** Trace
Format (Apalache ADR-015; the URL is in `#meta.format-description`).

Top level:

```json
{
  "#meta": { "format": "ITF", "source": "main.qnt", "timestamp": 1787241833337, "…": "…" },
  "vars": ["jamStagingSet","lastHeadRoot","lastStepAssigns","lastStepWorkResults",
           "logPrunedBelow","now","prevSvc","solicitedSet","svc"],
  "states": [ { "#meta": {"index": 0}, "svc": {...}, "now": {...}, "…": "…" } ]
}
```

Note: `vars` is **alphabetically sorted**, and each state is an **object keyed by
variable name** — not a positional array.

Value encoding:

| Quint | ITF JSON |
|---|---|
| `int` | `{"#bigint":"42"}` (always, even for small values) |
| `bool` | `true` / `false` |
| record `{ hashBytes: 1 }` | `{"hashBytes":{"#bigint":"1"}}` |
| variant `MkParaId(1)` | `{"tag":"MkParaId","value":{"#bigint":"1"}}` |
| `None` | `{"tag":"None","value":{"#tup":[]}}` |
| `Some(v)` | `{"tag":"Some","value":<v>}` |
| nullary variant `Unprovided` | `{"tag":"Unprovided","value":{"#tup":[]}}` |
| tuple `(a, b)` | `{"#tup":[<a>,<b>]}` |
| `List[a]` | `[<a>, …]` (plain JSON array) |
| `Set[a]` | `{"#set":[…]}` |
| `a -> b` | `{"#map":[[<k>,<v>], …]}` |

Worked example — one `parachains` entry as actually emitted:

```json
{"#map":[[
  {"tag":"MkParaId","value":{"#bigint":"1"}},
  {"headData":{"#bigint":"0"},
   "isDeregistering":false,
   "pendingUpgrade":{"tag":"None","value":{"#tup":[]}},
   "totalStateBalance":{"#bigint":"271140"},
   "usedStateBalance":{"#bigint":"135570"},
   "validationCode":{"tag":"Some","value":{
     "pinned":false,
     "ref":{"hash":{"vchBytes":{"#bigint":"1"}},"len":{"#bigint":"65536"}}}}}
]]}
```

`validationCode` is `Option[{ref: ValidationCodeRef, pinned: bool}]` — **nested**,
matching Rust's `Option<ValidationCode>` where `ValidationCode { code_ref, pinned }`.

Implementation consequence: deserialize into a small generic `ItfValue` enum
(`Int(i128) | Bool | Str | List | Set | Map | Tup | Variant{tag,value} | Record`)
with typed extractors, rather than hand-writing `#[derive(Deserialize)]` structs
per Quint type. The `#bigint`/`#set`/`#map`/`#tup` sigils make the generic form
strictly simpler.

---

## Frame classification

The three step kinds in `main.qnt:312` are **not** interchangeable:

| Quint action | runs `applyAlwaysAccumulate`? | Rust replay |
|---|---|---|
| `stepRefineAccumulate` | yes (`accumulate.qnt:611`) | `run_block(storage, work_items, slot)` |
| `stepIncomingTransfer` | **no** | see divergence **D-2** below |
| `provisionPreimage` | **no** | direct `Storage::provide`, no block |
| `expectThat` (tests.qnt only) | — | skip: exact no-op frame |

Rust's `accumulate()` runs `assigns::apply_due_assigns(now, …)` **unconditionally
and first** (`service/src/accumulate/mod.rs:35`). Calling `run_block` on a
transfer-only or preimage frame therefore flushes a due assign the model left
alone. The classifier is not optional.

Classification rules:

1. If `mbt::actionTaken` is present (fuzz traces), use it verbatim.
2. Otherwise (`quint test` traces): a frame whose every variable is identical to
   its predecessor is an `expectThat` no-op — **skip it**. Verified: `expectThat`
   frames are byte-identical across all vars.
3. Otherwise classify structurally: `lastStepWorkResults` non-empty → block;
   `now` unchanged and only `preimageStatus` changed → provision; `now` unchanged
   and only the transfer queue changed → incoming transfer.

Rule 3 is a fallback and must **assert** that exactly one classification matches,
failing loudly on ambiguity rather than guessing.

Slot: a block frame's timeslot is `states[i+1].now`, not `states[i].now` — the
model advances the clock *inside* the step (`blockNow = now + blockGap`).

---

## Divergence ledger

This is the part that makes "compare every field" honest. Each entry is a
**known, intentional** difference with a source reference and a normalization.
The harness applies the ledger, then requires exact equality. **Any difference
not in the ledger fails the test.**

### D-1 — Balance encoding width (SPEC_GAPS #4, DECISIONS.md D-3)

Quint sizes balances as `Compact<u128>` (17 B worst case,
`quint/state_balance.qnt:19`); Rust uses `Balance = u64` → 9 B
(`service/src/state_balance.rs:8-11`, which already carries
`TODO: needs upstreaming into the §6.1 tables`).

Computed both sides:

| constant | Quint | Rust | Δ (quint − rust) |
|---|---|---|---|
| ParaInfo entry | 4 262 | 4 246 | **+16** |
| `BaselineFootprint` | 69 847 | 69 831 | **+16** |
| incoming-transfer bucket | 204 | 196 | **+8** |
| Asset Hub global items | 1 442 660 | 1 434 664 | **+7 996** |
| Asset Hub baseline | 1 512 507 | 1 504 495 | **+8 012** |

The +16 is exactly the two `ParaInfo` balance fields (17+17 vs 9+9). The
+7 996 is 1 000 buckets × 8, less 4 for D-3 below.

The Quint figures reproduce the trace's init frame exactly — Coretime
`271140/135570`, Asset Hub `1713800/1578230` — so this is arithmetic, not
inference.

**Normalization — compare in baseline-relative units.** Define

```
shift(p) = quintBaseline(p) - rustBaseline(p)     // 16, or 8012 for Asset Hub
```

Every *delta* the two sides apply agrees exactly (`preimageFootprint`,
`kvEntryFootprint` — see the equivalence list below), and Asset Hub
pre-provisions its transfer buckets at registration rather than charging them
incrementally. So the divergence enters `used_state_balance` at exactly one
point: **when a side computes a baseline from scratch**, i.e. registration.

Therefore:

- **Seeding (frame 0)** — shift *both* fields down so headroom is preserved bit
  for bit:
  ```
  rust_used(p)  = quint_used(p)  - shift(p)
  rust_total(p) = quint_total(p) - shift(p)
  ```
  Shifting `total` too is the point: `total - used` is what gates every
  write-time check, and leaving `total` unshifted would hand Rust `shift(p)`
  units of free headroom at seeded paras.
- **Comparison** — the harness records the shift it applied per para (`0` for
  paras registered during the replay, since their `total` comes from the
  message) and asserts `quint_used(p) == rust_used(p) + shift_applied(p)`.

Concrete case this is load-bearing for: `paraRegisterOperateCleanupTest` asserts
`usedStateBalance == BaselineFootprint` after registering para 4 — 69 847 in the
model, 69 831 in Rust. Without this normalization that trace fails on its first
block for a reason that has nothing to do with the code under test.

**This is why the original plan's "every field, no subsetting" fails immediately.**

### D-2 — Always-accumulate on non-block steps

Rust runs always-accumulate on every `accumulate` invocation; Quint's
`stepIncomingTransfer` and `provisionPreimage` do not. In JAM, transfers are
delivered as accumulate operands, so **Rust is right and the model
under-specifies**.

**Normalization:** replay a `stepIncomingTransfer` frame as
`run_block(storage, vec![transfer_item(from, amount)], slot)`, then compare
everything *except* `pendingAssigns` / `pendingAssignCores`, which are instead
checked against the model's values advanced by an out-of-band
`applyAlwaysAccumulate`-equivalent. Flag in the ledger with the upstream issue
link.

### D-3 — Incoming-transfer chain counter (SPEC_GAPS #2)

Rust's `incoming_transfer_chain` carries a 4-byte count the model does not
(`service/src/state_balance.rs`, `transfer_chain = 34+1+1+4+4+4`; Quint
`state_balance.qnt`, `transferChain = 34+1+1+4+4`). Accounts for the −4 inside
D-1's Asset Hub figure.

**Normalization:** folded into D-1's offset; the chain *value* comparison ignores
the counter field.

### D-4 — Transfer admission threshold

Quint admits a transfer at a full queue iff
`amount >= IncomingTransferEntryFootprint` = **204**; Rust's is **196**. A
transfer with amount in `[196, 204)` is admitted by Rust and rejected by Quint.
Same root cause as D-1.

**Normalization:** none available — this is a *behavioural* divergence, not a
numeric one. The harness detects it and reports it as a known-divergence hit
rather than a failure. Reachability in practice is low (`stepIncomingTransfer`
samples amounts from `{0, 100, 10000}`), but assert it explicitly so it cannot
pass silently.

### D-5 — Headroom slack at mid-trace registration

For a para **registered during** the replay, `total_state_balance` comes from the
`ParachainSetStateBalance` message unshifted while `used_state_balance` starts at
Rust's smaller baseline. Rust therefore carries `shift(p)` units — 16, or 8 012
for Asset Hub — of headroom the model does not. At a balance cap this changes
behaviour, not just a number.

Shifting the message payload instead would tamper with the trace's input, so we
do not: the slack is recorded and asserted.

Reachability today is narrow but real. `solicitRejectedAtBalanceCapTest`
registers para 6 exactly at its cap and then solicits a 500-byte preimage
costing `187 + 500 = 687` — comfortably above the 16-unit slack, so the
rejection still fires and the trace passes. It would *not* pass for a solicited
payload under 16 units. The harness asserts the slack explicitly so this stays
visible rather than becoming a latent false pass.

### Verified as *equivalent* (no ledger entry needed)

Checked, agree exactly — do **not** add offsets for these:

- `preimageFootprint(len)` = `187 + len` on both sides.
- `kv_entry_footprint(k, v)` = `49 + compactLen(k) + k + compactLen(v) + v`.
- `CORE_COUNT` 341, `EXPUNGE_PERIOD` 19 200, `PARACHAIN_LOG_BYTE_CAP` 64 KiB,
  `STORED_AUTH_TRACE_CAP` 256, `AUTHORIZER_QUEUE_LEN` 80,
  `MAX_STAGED_VALIDATOR_KEYS` 1 023, `MAX_INCOMING_TRANSFERS` 1 000.
- `ParachainWorkDigest` shape: `Ok{para_id, validation_code, parent_head_hash,
  head_data, upward_messages, lookup_anchor} | Err{para_id, error}` — identical
  in `messages.qnt:203-217` and `service/src/work_digest.rs:25-48`.
- Log entry framing: 4 B timeslot + 1 B discriminant + body, both sides.

**(unverified)** Per-variant `accumulateLogSize` / `refineLogSize` vs Rust's
derived `encoded_size()`. Structurally aligned; exact per-variant equality is a
Phase 0 check. Low risk. One known edge: Quint's `compactLen` returns 5 for
`n ≥ 2^30`, while SCALE big-integer mode is `1 + byte-length`; only reachable
with implausibly large balances.

---

## Value mapping (`Codex`)

Quint's cryptographic values are abstract integers; Rust's are real bytes. One
module owns every mapping, in both directions, so the harness never
improvises.

| Quint | Rust | Mapping |
|---|---|---|
| `MkParaId(n)` | `ParaId(n as u32)` | identity |
| `MkServiceId(n)` | `ServiceId(n)` | identity |
| `Timeslot`, `CoreIndex`, `Balance` | `u32`/`u64` | identity (range-checked) |
| `HeadData = int` | `BoundedVec<u8, 4KiB>` | 8-byte LE of the int |
| `{ hashBytes: n }` | `Hash([u8;32])` | `blake2_256(blob(n))` |
| `{ vchBytes: n }` | `ValidationCodeHash` | `vchAsHash` then as above |
| `{ authBytes: n }` | `AuthorizerHash` | 4-byte LE, zero-padded |
| `authTrace: n` | `AuthTrace(vec![0xAA; n])` | `n` is a *length* proxy — `truncateAuthTrace` caps it at 256 (`messages.qnt:246`) |

`HeadData`: must handle `StaleParentHead = 9223372036854775807` (`i64::MAX`),
hence 8 bytes, not 4.

### The preimage-bytes constraint

`{hashBytes: n}` maps to `blake2_256(blob(n))`, so `blob(n)` must be **real
bytes** — `Storage::provide` takes the preimage, not the hash
(`service/bin/src/mock.rs:443`). `blob(n)` is defined as `len` bytes with the
8-byte LE of `n` at the front, zero elsewhere.

But the length is part of the identity: `blob` is keyed on `(n, len)` while the
model's hash is keyed on `n` alone. The model uses `{hashBytes: 99999}` at
`len = 4096` (`UpgradeService`) **and** `{vchBytes: 99999}` at `len = 999`
(`arbitraryCandidates`), and `vchAsHash` collapses those to the same `Hash`. No
byte string has one blake2 hash at two lengths.

**Rule:** `Codex` maintains `n -> len` on first use and **panics on a conflicting
length**. A `(n, len)` pair that cannot be materialized may only ever be
`Unprovided`. The harness asserts this: if a trace ever moves such a pair to
`Provided`, the run fails with a clear message rather than a hash mismatch 40
lines deep.

`serviceCodeHash` uses the same map. Its init value `{hashBytes: 0}` cannot be
seeded literally — `fresh_storage` needs the code hash to be `hash_raw(SERVICE)`
for the blob to execute. `Codex` therefore pins `0 <-> hash_raw(SERVICE)` as a
fixed entry.

---

## Seeding

`preimageStatus` is **not** a compare-only ghost. The model *reads* it —
`refine.qnt:190` gates on `codeAvailable`, and the two-step forget/expunge path
keys off `Unrequested(since)`. Rust reads the same state from JAM via `query()`
(`service/src/state_balance.rs:24`). If the initial status is not reproduced
exactly, the replay diverges for reasons that have nothing to do with the code
under test.

Seeding recipe per `(hash, len)`, using `jam_node::vm::Storage`:

| model status | calls |
|---|---|
| `Unprovided` | `solicit(0, SVC, h, len)` |
| `Provided` | `solicit(0, …)`, `provide(0, SVC, blob)` |
| `Unrequested(y)` | `solicit`, `provide`, `forget(y, …)` |
| `Rerequested(y)` | `solicit`, `provide`, `forget(y, …)`, `solicit` |

Traces rooted at `init` only ever start `Unprovided`/`Provided`, so the last two
rows are needed only if we later seed mid-trace. Implement all four anyway; they
are four lines each.

Everything else in the initial `svc` frame maps to service KV via
`common::set_state` + `storage_key(Tag::*, …)`. Tags exist for all of it
(`service/src/state/mod.rs:25-34`): `Parachains`, `ParachainLog`,
`PendingAssigns`, `PendingAssignCores`, `PreimageRegistry`,
`StagedValidatorKeys`, `IncomingTransfers`, `IncomingTransferChain`,
`KeyValueStorage`. `StagedValidatorKeys` and `IncomingTransferChain` are
singletons (key `()`).

---

## What is compared

After every replayed **block** frame, all of:

| Field | Quint path | Rust |
|---|---|---|
| head data | `.headData` | `ParaInfo.head_data` |
| validation code + pinned | `.validationCode` | `ParaInfo.validation_code` |
| pending upgrade | `.pendingUpgrade` | `ParaInfo.pending_upgrade` |
| total state balance | `.totalStateBalance` | `ParaInfo.total_state_balance` |
| used state balance | `.usedStateBalance` | `ParaInfo.used_state_balance` (ledger D-1) |
| deregistering | `.isDeregistering` | `ParaInfo.is_deregistering` |
| registered set | `svc.parachains` keys | `Tag::Parachains` |
| incoming transfers | `svc.incomingTransfers` | `Tag::IncomingTransfers` |
| transfer chain | `svc.incomingTransferChain` | `Tag::IncomingTransferChain` (ledger D-3) |
| parachain log | `svc.parachainLog` | `Tag::ParachainLog` |
| preimage registry | `svc.preimageRegistry` | `Tag::PreimageRegistry` |
| pending assigns | `svc.pendingAssigns` | `Tag::PendingAssigns` (ledger D-2) |
| pending assign cores | `svc.pendingAssignCores` | `Tag::PendingAssignCores` (ledger D-2) |
| staged validator keys | `svc.stagedValidatorKeys` | `Tag::StagedValidatorKeys` |
| KV storage | `svc.keyValueStorage` | `Tag::KeyValueStorage` |
| service code hash | `svc.serviceCodeHash` | JAM `Service.code_hash` |
| preimage request status | `svc.preimageStatus` | JAM `query()` per registry key |

`preimageStatus` **is** compared, not "best-effort": every
`(hash, len)` in the model's map is queried through the JAM storage mock and the
four-state status compared exactly.

### `lastHeadRoot` — compared structurally, not by bytes

Quint's `merkleHash` is abstract arithmetic —
`head_commitment.qnt:27`, `{hashBytes: 2*(paraId*257 + headHash)}` — while Rust
uses keccak-256 over SCALE (`service/src/head_commitment.rs:27-33`). The root
*bytes* are incomparable by construction.

`state_vars.qnt` is explicit that this is **not** ghost state: it is the
service's real accumulate output. So it is compared as far as it can be:

1. `Some`/`None` agreement (no head changed ⇒ no commitment).
2. The set of paras whose head changed, recovered by diffing `prevSvc.parachains`
   against `svc.parachains`, must equal the leaf set Rust's `HeadTracker` built.
3. Rebuild the tree in the harness from that leaf set using Rust's own
   `MerkleTree`/`pair_up` and require the root to match Rust's returned hash —
   which checks ordering and the D-12 odd-promotion rule against the model's
   *shape*, without needing hash agreement.

### Not compared

`prevSvc`, `lastStepAssigns`, `logPrunedBelow`, `solicitedSet`, `jamStagingSet`
are Quint bookkeeping with no Rust counterpart. `jamStagingSet` becomes
comparable if the mock exposes `designate` effects — **(unverified)**, deferred.

### Failure ergonomics

`assert_state_eq` returns a structured diff, not a `Debug` dump of two large
structs: first divergent field, the two values, the trace name, the frame index,
the Quint action that produced it, and whether a ledger entry was applied. A
whole-struct `assert_eq!` on this state is unreadable and will get muted.

---

## Implementation

### Phase 0 — the spike (do this first, ~half a day)

One test, no new crate, no generality: parse
`fixtures/quint/paraRegisterOperateCleanupTest.itf.json`, seed frame 0, replay
its first block, compare every field.

Run it **without** the D-1 shift first and confirm it fails by exactly 16 on
para 4's `usedStateBalance` (69 847 model vs 69 831 Rust). Then apply the shift
and confirm it passes. That is the ledger's arithmetic validated against a real
run rather than against my spreadsheet.

Exit criteria:
- unshifted run fails by exactly `shift(p)`, no more and no less — any other
  delta means there is a divergence not yet in the ledger;
- shifted run compares clean on every field;
- an artificially perturbed value still fails (the comparison is not vacuous).

### Phase 1 — Codex + parser

`ItfValue` + typed extractors; the `Codex` bijections with the conflicting-length
panic; round-trip unit tests (`quint int -> rust -> quint`).

### Phase 2 — seeding + classification

Initial-frame seeding including preimage status; the frame classifier with its
ambiguity assertion.

### Phase 3 — replay + compare

The per-kind replay and the structured diff. Land with
`codeUpgradeLifecycleTest` and `multipleWorkPackagesAccumulateTest` only.

### Phase 4 — breadth

The remaining eight `quint test` traces, then the fuzz trace from
`quint run --mbt`.

### Location — no new crate

The original plan proposed a `quint-trace` workspace crate to keep `serde` out of
the service. That is already achieved by a dev-dependency, and the crate does not
work as specified: it would need `jam-node` (for `Storage`) and
`parachain-service-bin` (for `mock::provide_preimage`, `MOCK_SERVICE_ID`) —
neither in its proposed manifest — while `service/bin` dev-depends on it.

`service/bin` already has the `test-utils` feature and a self dev-dependency that
pulls in `jam-node`, `executor`, `codec` and `jam-std-common`. So:

- `service/bin/tests/common/itf/` — `mod.rs`, `value.rs`, `codex.rs`,
  `seed.rs`, `classify.rs`, `compare.rs`, `ledger.rs`
- `service/bin/tests/quint_replay.rs` — the test entry point
- `service/bin/tests/fixtures/quint/*.itf.json` — normalized traces

Only new dependency, in `service/bin`'s `[dev-dependencies]`:

```toml
serde_json = "1"
```

`serde` derive is not needed — the generic `ItfValue` walks `serde_json::Value`.
`serde_json 1.0.151` is already in `Cargo.lock` (transitively), so this pulls
nothing new into the build.

(Note: the original plan's `serde_json = { default-features = false }` does not
build without `alloc`/`std`, and workspace deps belong in
`[workspace.dependencies]`, not `[dependencies]`.)

### Test naming

Per CLAUDE.md and every existing test file, the module name is not repeated in
the test function. In `quint_replay.rs`:

```rust
#[test] fn code_upgrade_lifecycle_works() { replay("codeUpgradeLifecycleTest"); }
#[test] fn stale_parent_candidate_rejected_works() { … }
```

One `#[test]` per trace, so a failure names the scenario.

---

## Justfile

`justfile_directory()` inside a `just` **module** resolves to the **root**
justfile's directory, not the module's — verified empirically with just 1.58.0.
The original plan's `../vendor/…` therefore escaped the repo. No `../`.

**`scripts/common.justfile`** — add:

```just
QUINT_SPEC := justfile_directory() / "vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint"
QUINT_FIXTURES := justfile_directory() / "service/bin/tests/fixtures/quint"
```

**`scripts/quint.justfile`** (new):

```just
import 'common.justfile'

# Regenerate the committed ITF fixtures from the vendored Quint spec.
# `#meta` is stripped: it embeds a wall-clock timestamp, so keeping it would
# make every regeneration a spurious diff.
trace-generate:
	#!/usr/bin/env sh
	set -eu

	mkdir -p "{{ QUINT_FIXTURES }}"
	tmp=$(mktemp -d)
	trap 'rm -rf "$tmp"' EXIT

	cd "{{ QUINT_SPEC }}"
	quint test --out-itf="$tmp/out_{test}_{seq}.itf.json" --seed=1 tests.qnt
	quint run --out-itf="$tmp/fuzz.itf.json" --seed=42 --max-steps=20 --mbt main.qnt

	# `quint test` names files `out_{test}_{seq}` where `{seq}` is a GLOBAL
	# counter over the whole run — it shifts whenever a test is added or
	# `--match` narrows the set, so it is stripped to keep fixture names stable.
	# `#meta` is dropped (top level and per-state) because it embeds a wall-clock
	# timestamp; `mbt::*` fields survive.
	#
	# Traces with no state transition (the 51 pure-value `tests.qnt` runs) carry
	# nothing to replay and are dropped. The count is printed, never silent.
	has_transition='.states as $s | any(range(0; ($s|length)-1); $s[.] != $s[.+1])'
	kept=0
	dropped=""

	for f in "$tmp"/*.itf.json; do
		name=$(basename "$f" | sed -E 's/^out_//; s/_[0-9]+\.itf\.json$/.itf.json/')
		jq 'del(."#meta") | .states |= map(del(."#meta"))' "$f" > "$tmp/norm.json"
		if jq -e "$has_transition" "$tmp/norm.json" >/dev/null; then
			mv "$tmp/norm.json" "{{ QUINT_FIXTURES }}/$name"
			kept=$((kept + 1))
		else
			dropped="$dropped ${name%.itf.json}"
		fi
	done

	echo "Wrote $kept fixtures to {{ QUINT_FIXTURES }}"
	echo "Dropped (no state transition):$dropped" | fold -s -w 100

# Replay the committed fixtures against the Rust accumulate pipeline.
trace-replay *ARGS:
	cargo test --package parachain-service-bin --test quint_replay {{ ARGS }}

# Regenerate then replay.
trace: trace-generate trace-replay
```

**Root `Justfile`** — add alongside the existing modules:

```just
mod quint 'scripts/quint.justfile'
```

Requires `quint` (0.32.0) and `jq` on `PATH`; both are dev-only.

Verified end to end: this recipe runs as written, keeps **11** fixtures (the 10
`tests.qnt` runs with a state transition, plus `fuzz.itf.json`), names them
stably, reports the 51 dropped runs by name, and produces **byte-identical**
output across repeated invocations.

Note: there is currently **no CI configuration in this repo** (no `.github/`,
no `.gitlab-ci.yml`). `just quint trace-replay` should be added to `just ci`
once the harness lands, and to a pipeline whenever one exists — but the original
plan's "CI and local dev both run the same target" describes something that does
not exist yet.

---

## Files changed

| File | Action | Notes |
|---|---|---|
| `Justfile` | Edit | `mod quint 'scripts/quint.justfile'` |
| `scripts/common.justfile` | Edit | `QUINT_SPEC`, `QUINT_FIXTURES` (no `../`) |
| `scripts/quint.justfile` | New | generate / replay targets |
| `service/bin/Cargo.toml` | Edit | `serde_json = "1"` in `[dev-dependencies]` |
| `service/bin/tests/common/itf/mod.rs` | New | module wiring + public entry points |
| `service/bin/tests/common/itf/value.rs` | New | `ItfValue` + extractors |
| `service/bin/tests/common/itf/codex.rs` | New | int ↔ hash ↔ preimage-bytes ↔ head-data |
| `service/bin/tests/common/itf/seed.rs` | New | initial-frame seeding incl. preimage status |
| `service/bin/tests/common/itf/classify.rs` | New | frame classifier |
| `service/bin/tests/common/itf/ledger.rs` | New | divergence ledger |
| `service/bin/tests/common/itf/compare.rs` | New | full-field compare + structured diff |
| `service/bin/tests/quint_replay.rs` | New | one `#[test]` per trace |
| `service/bin/tests/fixtures/quint/*.itf.json` | New | 11 normalized fixtures (`just quint trace-generate`) |
| `QUINT_REPLAY.md` | This file | |

No workspace-member change, no new crate.

---

## Out of scope

- **Refine equivalence.** The model's PVF is an oracle with no PoV; there is
  nothing to hand a real PVF. We consume `lastStepWorkResults` as Accumulate
  input. Note this includes mapping `WorkExecResult::WorkErr` (the
  JAM-substituted error from `PvfPanic`, `refine.qnt:194`) to
  `WorkItemRecord { result: Err(..) }` — the existing `common::work_item` helper
  only builds the `Ok` variant and needs an `Err` sibling.
- **Rust → Quint trace generation.** Separate effort.
- **Temporal invariants.** `quint verify` / `--invariant` checks those; this
  harness checks state equivalence per step.
- **`jamStagingSet`** until the mock exposes `designate` effects **(unverified)**.

## Upstream feedback generated by this work

Following the existing `upstream-feedback/` convention:

1. **`stepIncomingTransfer` / `provisionPreimage` skip always-accumulate.** JAM
   delivers transfers as accumulate operands, so a transfer-carrying block does
   run always-accumulate. See ledger **D-2**.
2. **`state_balance.qnt:19` sizes balances as `Compact<u128>`.** The
   implementation pins `Balance = u64` (DECISIONS.md D-3, SPEC_GAPS #4). Either
   the §6.1 tables move to `u64` or the divergence becomes permanent. See
   ledger **D-1**.
3. **51 of 61 `tests.qnt` runs emit no state transition**, so they cannot be
   replayed despite the file's stated purpose. Converting the highest-value ones
   to action-driven form (as `codeUpgradeLifecycleTest` already is) would
   multiply this harness's coverage at low cost.

## Open questions

- Do we fork the vendored spec to add a step-kind ghost var, or rely on
  `--mbt` + structural classification? Current plan: the latter — no fork.
- Which fuzz seeds do we commit? Proposal: one, regenerated deliberately, plus an
  opt-in `just quint trace --seeds=…` sweep that is not committed.
