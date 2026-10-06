# Quint Accumulate replay

The harness replays Quint ITF traces through Rust Accumulate in the PolkaJAM PVM.
It compares storage, head commitments, and supported JAM effects after each
transition. Coverage includes ordered outgoing transfers, regular JAM balances,
validator designation, core assignments, and service upgrades. Upgrade checks
verify installed code and gas settings; subsequent invocations execute that code.
Quint supplies work results; Rust Refine is not executed.

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
supervisor links are unavailable in the host; foreign accounts must already exist,
be self-supervised, and have zero supervisor balance. Negative regression tests
require specific failures in the pinned model's `solicit_implies_registry` and
`parent_head_continuity` predicates, even where Rust and model storage agree.
