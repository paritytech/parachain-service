# Accumulate Quint replay TODO

Coverage gaps identified on 2026-10-07. These concern generated Accumulate replay;
some cases already have deterministic replay fixtures or direct Rust tests.
See [QUINT_REPLAY.md](QUINT_REPLAY.md) and the
[fuzzing guide](service/bin/tests/fixtures/quint/FUZZING.md) for current coverage.

## Priority 1: recovery and storage failures

- [x] Fuzz invocation gas exhaustion before the first checkpoint, including due
  assignments and incoming-transfer processing. The independent Quint oracle
  retains scheduler credits and rolls back guest state and effects. Deterministic
  boundaries and mixed campaigns check subsequent invocations from recovered state.
- [x] Fuzz exhaustion after the final report checkpoint. Deterministic boundaries
  and mixed campaigns verify retained persisted state and effects, head-commitment
  handling, `NotEnoughGas` without a commitment, and subsequent invocations.
  The current runtime persists writes before checkpointing; its final tail has
  head reads and commitment computation, with no further storage writes.
- [x] Broaden the storage profile beyond KV writes: staged validator keys,
  registration, compact-encoded state-balance growth, forced heads, and forced
  validation code now have deterministic and generated replay coverage. Checks
  include retained state, partial effects, no removed allowance reasons, exact
  deposit boundaries, refunds, and retries.
- [ ] TODO: Extend storage-failure coverage to code announcements and failure
  sites during forced-code reference acquisition. The forced-code profile
  currently funds acquisition and tests rejection of the final metadata write.
- [ ] TODO: Mix head and incoming-queue write failures with general campaign
  actions and gas recovery. These failures currently have a separate storage
  profile, which excludes other upward messages, gas interruptions, and arrivals
  above reserved queue capacity.

## Priority 2: boundaries and oracle coverage

- [ ] TODO: Cover threshold-adjacent outgoing spending. First define an explicit
  compatibility oracle for the model/host threshold difference, then sample
  below, at, and above the actual spendable balance.
- [ ] TODO: Cover deferred self-payments. Model scheduler delivery after the
  invocation explicitly; the pinned model currently credits self immediately.
- [ ] TODO: Generate validator designation privilege rejection. Replay currently
  assumes designation privilege; direct Rust tests cover rejection. Compare
  logs, staged keys, and final designation effects across later invocations.
- [ ] TODO: Re-enable `solicit_implies_registry` once the model tracks only
  successful solicitations; restore its mutation-test coverage too. See the
  existing reference to issue #54 in
  [invariants.rs](service/bin/tests/quint_replay/itf/invariants.rs).

## Blocked on host capabilities

- [ ] TODO: Replay successful supervised-service operations: foreign preimage
  solicit/forget, storage removal, supervisor handoff, and ejection. Their
  reachable refusals are already fuzzed; the current host lacks mutable
  supervision.
- [ ] TODO: Replay nonzero supervisor balances and successful transfers using
  them, including incoming destination-balance selectors, once supported by the
  host. Preserve explicit rejection of unsupported traces until then.

These are test-coverage tasks, not a classification of the underlying production
risks. Existing consensus-critical `FIXME:` items remain tracked in source.
