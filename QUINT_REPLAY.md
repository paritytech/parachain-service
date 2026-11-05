# Quint Rust replay

The default campaign replays Quint inputs through Rust Refine and then Rust
Accumulate in the PolkaJAM PVM. It compares each actual refine digest or trap,
then feeds those actual results into accumulate and compares storage, head
commitments, and supported JAM effects after each transition. Coverage includes
ordered outgoing transfers, regular JAM balances,
validator designation, ordered successful core-assignment calls, service upgrades, service creation, and
ejection, service-targeted solicit/forget, storage removal, and supervisor handoff
refusals. Creation checks include the
new account and its code request. Upgrade checks
verify installed code and gas settings; subsequent invocations execute that code.
Historical fixtures and focused accumulate profiles still accept supplied work
results. The default campaign requires refine inputs for every candidate.
Recovery coverage
includes sampled invocation gas cutoffs before the first checkpoint, inside
reports, and after the final report checkpoint, transfer-induced out-of-gas, and
explicit malformed-digest faults that trigger real Accumulate
panics in the PVM. Gas calibration observes host-call order; expected state and
effects still come from Quint’s predetermined recovery oracle. Before the first
checkpoint, only incoming funds credited by JAM survive; due assignments and
incoming-queue writes roll back. After the final checkpoint, all report state and
effects survive, while the invocation returns `NotEnoughGas` and no commitment.
Later invocations retry from the recovered state.

- [Fixture guide](service/bin/tests/fixtures/quint/README.md): regeneration,
  coverage, and adapter limitations.
- [Fuzzing guide](service/bin/tests/fixtures/quint/FUZZING.md): campaigns,
  configuration, and saved-failure replay.
- [Harness](service/bin/tests/quint_replay): parsing, storage seeding, and checks.

The model is pinned by the `vendor/polkadot-sdk-quint` gitlink. Abstract hashes,
heads, code, and preimages are mapped to concrete Rust values. Head commitments
are checked against both the model tree and a reconstructed SCALE/Keccak tree.

Independent invariant checks inspect decoded Rust storage at initialization and
every frame, including no-ops, and validate head transitions and JAM mutations.
A catalogue test covers all 31 model predicates; a storage-key audit rejects
unknown or orphan entries. Traces must include `solicitedSet` and `logPrunedBelow`.
Invariant failures report the frame and predicate and fail replay, including fuzzing.

Replay rejects unsupported inputs and mismatches. Supervisor balances and mutable
supervisor links are unavailable in the host. Foreign accounts can be seeded or
created during replay, use self-supervision in the compatibility model, and have
zero supervisor balance. The host records the creator as parent but cannot
perform supervisor-driven ejection.

The Rust `parent_head_continuity` checker additionally snapshots deregistration
flags from pre-invocation storage: candidates from already deregistering parachains
and forced head updates targeting them are ignored. The pinned Quint predicate
omits this guard. The cleanup-retry fixture and fuzz seed `3123773523` cover the
correction; mutation tests still reject illegal head changes while deregistering.
`solicit_implies_registry` is enabled. The pin records successful explicit
solicit/forget operations; the replay wrapper additionally records implicit
reference releases during cleanup and code changes, preventing stale history
from reappearing when a parachain is re-registered. Mutation tests cover missing
entries, wrong referencers, and missing ghost metadata.

## Combined campaign

`just quint-fuzz --infinite` exercises both stages. Deploy with
`just quint-fuzz-deploy SERVER`; there is no separate refine campaign to select.
Each block retains its packages, ordered host-call scripts, code availability,
and lookup anchors alongside Quint's independently computed work results.
The harness executes refine sequentially for the block's candidates (currently
up to three), compares the results, and passes the actual decoded digests to
one accumulate invocation, preserving order. Explicit JAM work errors skip
refine. Deliberate malformed-output recovery faults are injected only after
refine has been checked, as part of the existing accumulate recovery tests.

The test-only `refine-replay-pvf` executes scripts through the real service host
functions. The currently installed service code is used, including after an
upgrade. The compiled interpreter PVF stands in for abstract validation code:
after verifying its hash in the real result, the adapter translates only that
identity back into the accumulate codex's candidate identity. Heads, messages,
errors, and lookup anchors are the actual Rust outputs. Code availability is an
input to the isolated historical-lookup context, reflecting the pinned model's
registry abstraction; arbitrary PVF semantics and historical availability
transitions are outside this model.

A predicted work failure requires an actual PVM trap, or actual gas exhaustion
in an explicit gas-fault scenario. Ordinary traces still reject out-of-gas;
invalid output and host initialization failures always fail replay. The dedicated
`refine_gas` campaign includes zero-entry, infinite-child, and sampled finite-script
gas cutoffs, funded candidates in the same block, and subsequent retries.
New campaign traces carry
`replayRefineRequired` so dropping refine inputs cannot silently downgrade a
saved trace into an accumulate-only replay. Legacy fixtures remain supported.

The model conservatively charges `ParachainSetHead` its maximum encoded size;
the existing codex maps abstract heads to empty or eight-byte heads. The default
message pools keep these calls away from the output-size boundary. Focused
refine boundary fixtures use concrete-size KV messages instead. Those standalone
fixtures remain regression tests, not a separate server profile.

The PolkaJAM hazmat host emits completed assignment, designation, upgrade, and
checkpoint events for ordered replay comparison. Assignment order is compared
against `lastStepAssigns`; interleaved designation/upgrade order is compared
against `replayHostCalls`. Completed checkpoints select the retained history
after rollback. Final-effect checks remain independent. New generated traces
require the history metadata; legacy traces retain their existing comparisons. Remaining work
is tracked in [the coverage TODO](QUINT_REPLAY_COVERAGE_TODO.md).
