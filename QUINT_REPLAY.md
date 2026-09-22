# Quint Accumulate replay

The harness replays Quint ITF traces through Rust Accumulate in the PolkaJAM PVM,
compares storage after each transition, and checks returned head commitments and
supported JAM effects. Quint supplies work results; Rust Refine is not executed.
Unsupported inputs and mismatches fail explicitly.

- [Fixture guide](service/bin/tests/fixtures/quint/README.md): deterministic
  regeneration, coverage, and adapter limitations.
- [Fuzzing guide](service/bin/tests/fixtures/quint/FUZZING.md): streaming campaigns,
  configuration, and saved-failure replay.
- [Harness](service/bin/tests/quint_replay): ITF parsing, value mapping, initial
  storage seeding, frame classification, and state/output comparison.

The model is pinned by the `vendor/polkadot-sdk-quint` gitlink. The value mapper
recovers hash domains from field context and translates abstract heads, code, and
preimages into concrete bytes. Head commitments are checked against both the
abstract model tree and a separately reconstructed SCALE/Keccak tree.

Rust currently matches Quint's omission of code-release notifications on upgrade
expiry and activation; [issue #36](https://github.com/paritytech/parachain-service/issues/36)
tracks preserving those logs. Rejected candidates preserve state, including logs
and pending upgrades, following Quint `06c2a49202`
([issue #35](https://github.com/paritytech/parachain-service/issues/35)).
