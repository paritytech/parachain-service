# Streaming Quint replay

The opt-in Rust test starts a persistent Node/Quint process per worker. Quint
chooses inputs and computes expected states; Rust replays the resulting work
results through Accumulate in the PVM and compares storage after every transition.
Incoming-transfer actions retain their actual operands and replay them through
the same PVM entry point, including operands the model drops.
After each block it also checks the returned head commitment and rejects JAM
host effects outside the supported input domain (see [README.md](README.md)).
Rust Refine is not executed. `fuzz.qnt` defines the input domain, not a sequence
of actions or expected states. It calls the pinned model's `refine`,
`accumulateBlock`, and `provisionPreimage` implementations.

Run from the repository root with Node and Quint **0.32.0** on PATH:

```sh
QUINT_FUZZ_TRACES=100 QUINT_FUZZ_STEPS=30 QUINT_FUZZ_WORKERS=4 \
  cargo test -p parachain-service-bin --test quint_replay \
  fuzz::generated_traces_works -- --ignored --nocapture
```

Configuration:

| Variable | Default | Meaning |
| --- | --- | --- |
| `QUINT_FUZZ_TRACES` | `100` | Total traces across workers; `0` runs until failure/interruption |
| `QUINT_FUZZ_STEPS` | `30` | Transitions per trace, 1–10000 |
| `QUINT_FUZZ_WORKERS` | `1` | Independent Rust workers and generator processes |
| `QUINT_FUZZ_SEED` | `1` | First seed; worker i uses seed+i, then increments by worker count |
| `QUINT_FUZZ_FAILURE_DIR` | `target/quint-fuzz` | Saved mismatch reports |
| `QUINT_PACKAGE` | resolved from `quint` on PATH | Optional installed npm package directory |

For an indefinite campaign, set `QUINT_FUZZ_TRACES=0`. Choose workers based on the
available CPUs and memory: each worker owns a Node process as well as a Rust
thread. `just quint-fuzz` uses the `testnet` Cargo profile: release optimizations
with debug assertions and overflow checks enabled. Add `--profile testnet` to
the Cargo commands here to use the same profile. The service blob already uses
its production build profile. The current PVM helper uses the
interpreter. This remains an integration test; a separate executable can reuse
the parsed-document replay entry point later.

## Transport and reproducibility

No trace files are written during successful generation/replay. Each generator
parses and typechecks the model once, then emits newline-delimited JSON envelopes
through its stdout pipe. Each envelope contains `seed`, `steps`, `version`,
`generation_ms`, and the complete ITF `trace`. Rust parses that document once and
passes it directly to the existing comparator. The OS pipe and Node's awaited
write backpressure bound the queue; generation can overlap Rust replay. There
are no named pipes, temporary trace directories, or CLI launches per trace.
Memory remains proportional to trace length and worker count, not campaign length.

Quint 0.32.0's CLI does not expose this transport, so
`scripts/quint-replay-stream.cjs` uses the CLI's internal TypeScript simulator and
ITF converter. It requires exactly that version. The `stream_matches_cli_works`
test compares two streamed seeds with independent CLI invocations, including a
second trace from the same persistent process. A future Quint update must check
this adapter explicitly. The simulator is invoked once per trace; the model
parse/typecheck is reused. This is not the Quint Rust simulation backend.

Workers stop on a mismatch, unsupported input, or replay panic, and terminate
their generators. A mismatch report contains the complete generated trace, seed,
limits, version, error, and repository/gitlink revisions. Replaying that report
does not require Quint and does not regenerate a potentially changed model:

```sh
QUINT_REPLAY_TRACE=target/quint-fuzz/failure-PID-SEED.json \
  cargo test -p parachain-service-bin --test quint_replay \
  fuzz::replay_input_works -- --ignored --nocapture
```

The replay input test also accepts an ITF document or a single stream envelope
on stdin. For example, this generates and replays one trace without trace files:

```sh
node scripts/quint-replay-stream.cjs \
  service/bin/tests/fixtures/quint/fuzz.qnt 1 1 1 10 |
  cargo test -p parachain-service-bin --test quint_replay \
    fuzz::replay_input_works -- --ignored --nocapture
```

Adapter arguments are model path, first seed, seed stride, trace count (0 means
unbounded), and steps. Its stdout is exclusively the JSON protocol; diagnostics
go to stderr. Rust aggregates successful replays across all workers into one
progress line, at most once every five seconds as traces complete. It reports
traces/s and transitions/s since the previous update, total completed traces and
transitions, and elapsed wall time. The final summary reports average throughput
over the whole run. Quint's per-simulation terminal bar is disabled. First-use
blob building is included in timing, so initial figures are not steady-state
benchmarks.

## Initial input domain and known findings

The generator samples zero to three WPs per block, both registered parachains,
valid candidates, stale parents, invalid code, missing head declarations,
reported PVF errors, PVF panic, JAM WorkErr, auth-trace lengths, time gaps, and
lookup anchors. It also samples external provision of solicited preimages.
Each WP independently samples its para, outcome, auth-trace length, and lookup
anchor. Same-para WPs either compete for the pre-block head or chain a candidate
off the latest preceding same-para candidate's proposed head. All refine against pre-block
state; Quint decides which results accumulate successfully. Empty blocks are
sampled independently of work outcomes. Mode 0 selects active-code work; mode 8 selects pending-code work when an
upgrade exists, otherwise active code. Mode 6 still samples arbitrary candidates.
Selecting whole outcome classes keeps successful candidates reachable frequently.
Time gaps are at most `MaxLookupAge`, so sampled anchors lie between the valid
lookback floor and the previous block slot.

Each WP independently samples zero to four upward messages from `Solicit`,
`Forget`, `RequestCodeUpgrade`, `SetKV`, `RemoveKV`, and
`ParachainSetStateBalance`. Two shared hashes at length 1024 exercise duplicate requests,
shared references, refunds, and provision/forget/re-solicit lifecycles. Active-code
messages exercise pinning and unpinning. Forget targets include the caller and
both registered paras, allowing Quint Refine to reject unauthorized foreign
calls. Messages are also sampled for failed work and stale-parent candidates;
Quint determines whether they reach Accumulate and take effect. The codex requires
one length per abstract hash, so conflicting lengths are excluded.

Upgrade requests sample two shared code hashes (777 and 778, each `FixedCodeLen`)
and the caller's active code. The same new hashes can be solicited and forgotten,
so requests can encounter independently held references and pinned pending code.
Requests can repeat, refresh a deadline, supersede a different upgrade, or fail a
reservation. External provision and pending-code candidates allow activation;
time gaps can cross upgrade deadlines. Expected outcomes always come from Quint.

KV messages share three keys across both paras, including an empty key. Values
include empty, same-length replacements, and 63/64-byte values spanning a SCALE
compact-length boundary. Removal targets include self and both registered paras,
exercising delegated removal and unauthorized work alongside writes. Ordering
can refund storage before another reservation, overwrite the same key, or leave
writes unapplied when work fails. Storage contents, absence, balances, and KV
failure-log key hashes are compared strictly. Keys avoid the leading-zero
collisions in the model's abstract `listHash`.

Balance updates target both registered paras and sample zero, one below current
usage, exact usage, one above usage, the current total, and enough headroom for
code reservations. Both paras can emit these messages, letting Quint reject
unauthorized callers. Their position within a WP and block varies alongside
writes, refunds, solicitations, and upgrade requests.

Incoming actions sample batches of one to three transfers, three source service
IDs, distinct integer memos, and amounts below/at/above the per-entry footprint
as well as zero and 10000. Each action records `replayIncoming` independently of
expected state, so replay checks rejected arrivals too. Source IDs are literal
u32 values; memo integers map to a u64 little-endian prefix padded to 128 bytes.
Only regular-balance transfers are generated: the vendored JAM host has no
supervisor-balance selector, and the adapter rejects that unsupported input.
These arrivals queue records and can charge Asset Hub's used state balance;
they do not directly top up a parachain's state allowance. Queue contents,
ordering, endpoints, count, and orphan storage keys are compared. Deterministic
`balances` fixtures additionally cross the bucket capacity and reservation limit.
The host service balance itself is not modeled by this replay profile.

The streaming initializer is `replayInit` (model `init` plus an empty operand
list); `replayStep` clears operands on block/provision actions. CLI parity tests
use the same initializer.

Registration and cleanup are not fuzzed yet. Malformed
authorizer configuration and invalid item counts are excluded because their
model Refine-log representations cannot be replayed as Rust Refine errors.
Other comparator limitations remain those in [README.md](README.md). Unsupported
values fail explicitly; the runner does not discard failing traces.

Rust matches the model by omitting `ForgetAgainAt` when an accepted candidate
releases provided code during expiry or activation, pending issue #36. Those
inputs remain enabled, and ordinary regression tests assert strict agreement. See [issue #36](https://github.com/paritytech/parachain-service/issues/36).

Quint `06c2a49202` changed rejection to preserve state. Rust now matches it:
rejected candidates neither prune logs nor commit tentative expiry cleanup.
The regenerated `log_pruning/stale_parent_seed_1_works.itf.json` covers the
rejection/pruning shape found by the old campaign; it no longer contains the
old model's expected states.

A small machinery check that precedes those failures, plus CLI parity:

```sh
QUINT_FUZZ_TRACES=2 QUINT_FUZZ_STEPS=10 QUINT_FUZZ_WORKERS=2 \
  cargo test -p parachain-service-bin --test quint_replay fuzz:: \
  -- --ignored --skip replay_input_works --nocapture
```
