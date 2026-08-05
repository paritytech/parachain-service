# Parachain Service PoC

The parachain service lets you run Polkadot parachains on JAM. It:

- Replaces the *candidate inclusion* with an implementation of JAM `accumulate`.
- Replaces the off-chain validation of *candidate backing* with an implementation of JAM `refine`.
- Offloads *approval checking*, *availability* and *backing* to JAM.
- Exposes a new Cumulus interface for collators to author parachain blocks as Work Packages.

There is no longer a relay chain runtime; the former relay chain logic is implemented without FRAME
directly as an `accumulate` hook.

## Project Structure

```
.
├── authorizer          # JAM authorizer for the service
│   └── bin             # Blob builder for the authorizer
├── cumulus             # Re-exports for candidate authorship
├── runtimes            # Parachain runtimes
│   └── frameless       # One mock runtime for both Coretime and Asset Hub
├── scripts             # Justfile modules
├── service             # The parachain service (refine + accumulate)
│   └── bin             # Blob builder for the service
├── support             # Code shared across the crates above
└── tools
    └── executor        # PolkaJAM test adapter
```

## Notable Code Locations

- [is_authorized.rs](./authorizer/src/is_authorized.rs) and its [tests](./service/bin/tests/is_authorized.rs)
- [refine.rs](./service/src/refine.rs) and its [tests](./service/bin/tests/refine.rs)
- [accumulate.rs](./service/src/accumulate.rs) and its [tests](./service/bin/tests/accumulate.rs)
- [frameless](./runtimes/frameless/src/lib.rs), the mock runtime whose `Config` picks Coretime or Asset Hub
- [PolkaJAM](./tools/executor/src/polkajam.rs) in-memory execution wrapper

## Executor

The blob integration tests use [PolkaJAM's in-memory executor](./tools/executor/src/polkajam.rs)
with real PVM blobs and host calls.

## Building

Fetch the vendored submodules once, then build and test with `cargo`:

```sh
git submodule update --init # only needed once
cargo test
```

There are [just](https://github.com/casey/just) recipes for the other operations (`just --list`).
