# Parachain Service PoC

Run Polkadot parachains on JAM: `refine` validates candidates, `accumulate`
handles inclusion and service state, and Cumulus lets collators author Work Packages.
JAM provides backing, availability, and approval checking without a relay-chain runtime.

## Build and test

```sh
git submodule update --init
cargo test
```

Run `just --list` for build and maintenance recipes. For Quint equivalence testing:

```sh
just quint-fuzz            # 100 traces, eight workers
just quint-fuzz --infinite # run until failure or interruption
```

See [Quint replay](QUINT_REPLAY.md) for prerequisites, fixtures, and failure replay.

## Code and references

- [Service](service/src/lib.rs): [Refine](service/src/refine.rs) and
  [Accumulate](service/src/accumulate/mod.rs), with [integration tests](service/bin/tests).
- [Authorizer core](authorizer), with [ed25519](authorizer-ed25519) and
  [sr25519](authorizer-sr25519) verifiers.
- [Cumulus interface](cumulus) and [mock parachain runtime](runtimes/frameless).
- [PolkaJAM executor](tools/executor/src/polkajam.rs) for PVM blob tests.
- [Pinned design and Quint model](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam).
- [Implementation decisions](DECISIONS.md), [divergences](DIVERGENCE.md), and
  [genesis setup](GENESIS.md).
