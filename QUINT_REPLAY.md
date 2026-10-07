# Quint Accumulate replay

The harness replays Quint ITF traces through Rust Accumulate in the PolkaJAM PVM.
It compares storage, head commitments, and supported JAM effects after each
transition. Coverage includes ordered outgoing transfers, regular JAM balances,
validator designation, core assignments, service upgrades, service creation, and
ejection, service-targeted solicit/forget, storage removal, and supervisor handoff
refusals. Creation checks include the
new account and its code request. Upgrade checks
verify installed code and gas settings; subsequent invocations execute that code.
Quint supplies work results; Rust Refine is not executed. Recovery coverage
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
`solicit_implies_registry` remains disabled pending issue #54.
