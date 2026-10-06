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
