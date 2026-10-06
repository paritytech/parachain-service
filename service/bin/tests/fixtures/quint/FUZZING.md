# Streaming Quint replay

Each worker runs a persistent Node/Quint generator and replays its traces through
Rust Accumulate in the PolkaJAM PVM interpreter. Quint supplies expected states
and work results; Rust Refine is not executed. Storage, head commitments, supported
JAM effects, and invariants are checked by the [replay harness](../../../../../QUINT_REPLAY.md).

## Running campaigns

Run from the repository root with Node and Quint **0.32.0** on PATH:

```sh
QUINT_FUZZ_TRACES=100 QUINT_FUZZ_STEPS=15 QUINT_FUZZ_WORKERS=4 \
  cargo test -p parachain-service-bin --test quint_replay \
  fuzz::generated_traces_works -- --ignored --nocapture
```

Configuration:

| Variable | Default | Meaning |
| --- | --- | --- |
| `QUINT_FUZZ_PROFILE` | `fuzz` | `fuzz` for general inputs; `storage` for host storage-budget failures |
| `QUINT_FUZZ_TRACES` | `100` | Total traces across workers; `0` runs until failure/interruption |
| `QUINT_FUZZ_STEPS` | `15` | Transitions per trace, 1–10000 |
| `QUINT_FUZZ_WORKERS` | `1` | Independent Rust workers and generator processes |
| `QUINT_FUZZ_SEED` | `1` | First seed; worker i uses seed+i, then increments by worker count |
| `QUINT_FUZZ_FAILURE_DIR` | `target/quint-fuzz` | Saved mismatch reports |
| `QUINT_PACKAGE` | resolved from `quint` on PATH | Optional installed npm package directory |


`just quint-fuzz` runs 100 traces with eight workers; `just quint-fuzz --infinite`
runs until failure or interruption. Set `QUINT_FUZZ_STEPS=30` for longer traces.
Each worker owns a Node process and a Rust thread, so choose worker counts to fit
available CPU and memory.

The recipe uses the `testnet` Cargo profile, with release optimizations, debug
assertions, and overflow checks. Add `--profile testnet` to the Cargo commands
here for the same settings. The service blob uses its production build profile.
Progress reports aggregate successful traces and transitions across workers at
most every five seconds. Timing includes first-use blob compilation.

## Failure replay

Workers stop on a mismatch, unsupported input, or replay panic. Reports contain
the complete trace, seed, limits, Quint version, error, and repository/gitlink
revisions. Replay a saved report without Quint:

```sh
QUINT_REPLAY_TRACE=target/quint-fuzz/failure-PID-SEED.json \
  cargo test -p parachain-service-bin --test quint_replay \
  fuzz::replay_input_works -- --ignored --nocapture
```

The replay test also accepts an ITF document or a stream envelope on stdin:

```sh
node scripts/quint-replay-stream.cjs \
  service/bin/tests/fixtures/quint/fuzz.qnt 1 1 1 10 |
  cargo test -p parachain-service-bin --test quint_replay \
    fuzz::replay_input_works -- --ignored --nocapture
```

Adapter arguments are model path, first seed, seed stride, trace count (`0` for
unbounded), and steps. Stdout carries JSON; diagnostics go to stderr.

## Input coverage

[`fuzz.qnt`](fuzz.qnt) samples inputs and calls the pinned model for outcomes.
Blocks contain zero to three work packages, with valid and invalid candidates,
stale parents, missing head declarations, PVF errors and panics, JAM work errors,
auth traces, and lookup anchors. Packages can compete for a head or chain off
preceding candidates. Blocks independently sample zero to three incoming transfers in the same invocation.
Due assignments flush first, arrivals are recorded next, and reports execute last.
Up to four upward messages per package exercise ordering,
authorization, and rejected-work behavior.

Per-report gas limits sample zero, one below the required cost, the exact cost,
one above it, and `u64::MAX`. The replay-only `gas_inputs.qnt` layer models the
§5.1 gate (5,000,000 base + 250,000 per message + deferred transfer gas, saturated
at `u64::MAX`); admitted reports use the unchanged pinned transition functions.
An empty `replayGasLimits` list selects unlimited budgets for legacy actions.

A dedicated action interrupts the first, middle, or last of three chained reports.
The failing report updates its head and KV, then forwards 6,000,000,000 gas into
a deferred transfer, exhausting the mock invocation's 5,000,000,000 gas pool.
The harness requires an actual VM out-of-gas error and applies JAM's checkpoint
recovery. Quint predicts the completed prefix, including incoming transfers and
due assignments; comparisons cover storage, balances, logs, and host effects.
The failed report and the remaining suffix must leave no effects. No head
commitment is returned on interruption. Subsequent frames replay from recovered
state. This covers transfer-induced exhaustion; arbitrary instruction-level gas
cutoffs and Accumulate panics are not yet sampled.

| Area | Sampled inputs |
| --- | --- |
| Preimages and parachain code | Shared solicitations, provision, forget/re-solicit, announcements, and upgrade application |
| Key-value storage | Shared keys, empty inputs, replacement, removal, authorization, and storage-cost boundaries |
| State balances | Values below, at, and above usage; reservation headroom and unauthorized updates |
| Incoming transfers | Batches, multiple sources and memos, zero amounts, and per-entry reservation boundaries |
| Lifecycle | Registration, funding, deregistration, delayed cleanup, and re-registration for ordinary paras |
| Core assignments | Queue-size and rotation boundaries, handoffs, and unauthorized callers |
| Validator keys | Partial/final chunks, empty aborts, assembled-length boundaries, and repeated calls |
| Outgoing transfers | Source/destination authorization, ordered payments, overdrafts, zero amounts, and memo-gas boundaries |
| Service upgrades | Funding and code preparation, unavailable code, wrong lengths, repeated upgrades, gas changes, and rejected work |

Dedicated actions exercise lifecycle, assignments, validator keys, outgoing
transfers, and service upgrades alongside mixed-message blocks. External actions
provide solicited preimages and incoming transfers. Replay retains incoming
operands even when the model rejects them and checks ordered outgoing records
and regular JAM balances. Service-upgrade checks include installed code and gas
settings; subsequent invocations execute the installed binary.

Initialization uses the model's service state, empty input/effect lists, and two
empty self-supervised foreign services (IDs 0 and 7, minimum memo gas 100 and 200).
Minimum accumulate gas starts at 100 and follows accepted upgrades.

Malformed authorizer configuration and invalid item counts are excluded because
their model log representations cannot be mapped to Rust Refine errors. Abstract
hashes use one length each; KV keys avoid model hash collisions. Host, transfer,
and comparison limits are documented in the [fixture guide](README.md).
Unsupported inputs and invariant failures fail replay; failing traces are not discarded.

## Generator checks

`scripts/quint-replay-stream.cjs` uses Quint 0.32.0's internal TypeScript simulator
and ITF converter. Each worker parses and typechecks once, then emits one JSON
envelope per trace. Pipe backpressure bounds buffering; successful traces are
not written to disk. Memory scales with trace length and worker count.

The CLI parity test compares streamed traces with independent CLI runs. Recheck
the adapter when changing Quint versions. Run the generator and coverage checks:

```sh
QUINT_FUZZ_TRACES=2 QUINT_FUZZ_STEPS=10 QUINT_FUZZ_WORKERS=2 \
  cargo test -p parachain-service-bin --test quint_replay fuzz:: \
  -- --ignored --skip replay_input_works --nocapture
```

## Mixed host budgets in the default campaign

The default `fuzz` profile now samples `hostBlock` alongside ordinary blocks,
provisioning, assignments, and lifecycle actions, preserving the same service
state between them. These invocations mix real KV backstop rejection with gas
limits, incoming transfers, due assignments, code announcements/application,
service upgrades, forget, KV removal, and queue cleanup. Reports can free
storage that later reports use. An optional final report forwards a zero-amount
transfer with excessive gas, requiring real PVM exhaustion and checkpoint
recovery even while the service has little free balance.

`host_invocation.qnt` extends the pin at KV writes and log persistence;
`host_sizes.qnt` independently calculates SCALE/JAM footprint changes, including
existing logs and pending assignments. `replayHostBudget` is free balance injected
before the invocation (`-1` means ordinary execution). Replay checks the expected
`replayHostFree` after execution or checkpoint rollback, then restores only the
injected balance offset. It retains real incoming credits, all storage changes,
and other host effects. This permits subsequent ordinary actions and installed
service-code replay without resetting service state.

The mixed action samples allowances of 1000, 2000, 4096, and 5000 and 4 KiB KV
writes. Its other message types must fit their modeled budget; unsupported
backstop sites disable that generated action rather than assuming success.
Head and incoming-queue write rejection remain covered by the isolated `storage`
profile below. Arbitrary low-balance handling for every upward message is not
modeled by the mixed action.

`host_mixed.qnt` has deterministic cross-feature cases, including successful
code/service upgrades before exhaustion and a failed KV write followed by a
successful retry after space is freed. `fuzz::host_budget_generated_works` checks
that sampled failures overlap arrivals and recovery, that gas gates reject work,
and that ordinary actions resume afterward. All cases use the same replay and
invariant checks as the general campaign.

## Host storage-budget profile

Run `QUINT_FUZZ_PROFILE=storage just quint-fuzz` (or set that variable on the
campaign command above). This uses `storage_fuzz.qnt` and the same streaming
workers, PVM replay, failure artifacts, and invariant checks. The ignored
`fuzz::storage_generated_works` regression asserts rejection coverage across
10 seeds and 300 transitions.

Before each invocation, an explicit environment input sets the actual JAM
balance to the current host threshold plus a sampled allowance. Incoming
amounts are then credited normally. Private parachain reservations remain
funded, isolating the host backstop from the private headroom check. Expected
free balance, write outcomes, logs, and partial effects are calculated in
`storage_inputs.qnt`, independently of Rust's storage encoders and write calls.
The extension fixes the pinned host's deposit parameters (10 per item, 1 per
byte, 34 bytes overhead) and SCALE sizes for its restricted input vocabulary.
It does not change the vendored Quint model.

Each invocation mixes zero to three arrivals with two reports, choosing
competing or chained parents, empty/eight-byte heads, and KV values of 1, 63,
64, or 256 bytes. It covers failed head growth followed by message execution,
KV charge rollback, bucket/endpoint rejection and cleanup, log write rejection,
and subsequent reports and invocations. Deterministic fixtures also reject a
second incoming bucket after the first was written. Storage/domain invariants
remain enabled; leaked buckets are failures. The profile currently excludes
other upward messages, gas interruptions, and transfers above the reserved
queue capacity.

## Creation, ejection, and service preimages

The default `fuzz` profile mixes `CreateService` and `EjectService` into ordinary
message pools and a dedicated action with incoming transfers and gas limits.
The same pools sample service-targeted `Solicit` and `Forget`, including unknown,
self, seeded, and newly created targets. Mixed host-budget actions include both
preimage operations, with expected refusal logs and no changes to requests.
Mixed host-budget actions also create services, exercising actual insufficient
funds, successful funding, and recovery alongside KV write rejection.

`services.qnt` covers repeated public allocations, protected ID collisions,
ignored public desired IDs, all balance-selector refusals, ejection of self,
unknown and existing services, transfers to created accounts, unauthorized
origins, gas rejection, and creation rollback at first/middle/last checkpoints.
`fuzz::services_generated_works` checks that the general campaign reaches both
creation and the host's reachable refusal classes. All traces execute the same
PVM replay and compare actual account/request effects, including after recovery.

The host has parentage but no supervision API, so successful supervisor ejection
and foreign preimage changes are outside the replay domain.
`service_preimages.qnt` adds deterministic refusal, authorization, stale-work,
gas, checkpoint, and host-budget cases. `fuzz::service_preimages_generated_works`
requires all four reachable solicit/forget refusal classes in the general campaign.
See [README.md](README.md) for the explicit host
compatibility rules, including the pinned host's debit on `IdTaken`.

`service_management.qnt` covers storage removal and supervisor handoff with the
same authorization, gas, checkpoint, and host-budget scenarios. The general
message pool includes removal targets and pairs of target/new-supervisor IDs,
including self, unknown, seeded, and created services.
`fuzz::service_management_generated_works` requires both store refusal classes
and all three handoff refusal classes. No refused operation may change foreign
storage, requests, or the model's supervisor links.
