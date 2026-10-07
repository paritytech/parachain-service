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
- [x] Extend storage-failure coverage to code announcements and forced-code
  reference acquisition. The `code_storage` profile checks charge rollback and registry
  rejection, real JAM solicit failure with checkpoint recovery, final metadata
  rejection, exact boundaries, shared references, and retries.
- [x] Mix head and incoming-queue write failures with general campaign actions
  and gas recovery. Mixed host budgets retain incoming credits on rejection,
  continue messages after failed heads, and cover queue-capacity boundaries,
  cleanup, later invocations, and explicit failed-head mutation checks.

## Priority 2: boundaries and oracle coverage

- [x] Cover threshold-adjacent outgoing spending. The `outgoing_boundary`
  profile substitutes independently tracked host free balance into the pinned
  transfer oracle, sampling below, at, and above spendable balance, including
  checkpoint recovery and subsequent invocations.
- [x] Cover deferred self-payments. The `self_payment` profile debits immediately
  and delivers retained payments after the invocation, checking ordered spending,
  refusals, checkpoint rollback, later credit, and subsequent invocations.
- [x] Generate validator designation privilege rejection. The `designation`
  profile varies host privilege between invocations and compares rejection logs,
  staged keys, aborts, invalid inputs, and restored designation effects.
- [ ] TODO: Re-enable `solicit_implies_registry` and restore its mutation-test
  coverage. The pinned model now tracks successful preimage operations in order;
  audit the historical fixtures before enabling the check globally. See the
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
