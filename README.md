# Parachain Service PoC

The parachain service lets you run Polkadot parachains on JAM. It:

- Replaces the *candidate inclusion* with an implementation of JAM `accumulate`.
- Replaces the off-chain validation of *candidate backing* with an implementation of JAM `refine`.
- Offloads *approval checking*, *availability* and *backing* to JAM.
- Exposes a new Cumulus interface for collators to author parachain blocks as Work Packages.

There is no longer a relay chain runtime; the former relay chain logic is implemented without FRAME
directly as an `accumulate` hook. Parachain runtimes need to use a new parachain system pallet and
for Asset Hub and Coretime there are special system pallets.

## Project Structure

```
.
├── authorizer          # JAM authorizer for the service
├── authorizer-bin      # Prebuilt-blob wrapper crate for the authorizer
├── pallets             # System pallets wired into the runtimes
│   ├── asset-hub-system
│   └── coretime-system
├── runtimes            # Parachain runtimes
│   ├── asset-hub
│   └── coretime
├── scripts             # Justfile modules
├── service             # The parachain service (refine + accumulate)
├── service-bin         # Prebuilt-blob wrapper crate for the service
├── support             # Code shared across the crates above
└── tools
    ├── executor        # PVM/runtime executor library
    └── executor-cli    # Debug CLI around the executor
```

## Notable Code Locations

- [is_authorized.rs](./authorizer/src/is_authorized.rs) and its [tests](./service-bin/tests/is_authorized.rs)
- [refine.rs](./service/src/refine.rs) and its [tests](./service-bin/tests/refine.rs)
- [accumulate.rs](./service/src/accumulate.rs) and its [tests](./service-bin/tests/accumulate.rs)
- [asset-hub](./runtimes/asset-hub/src/lib.rs) with its [system pallet](./pallets/asset-hub-system/src/lib.rs)
- [coretime](./runtimes/coretime/src/lib.rs) with its [system pallet](./pallets/coretime-system/src/lib.rs)
- [PolkaJAM](./tools/executor/src/polkajam.rs) in-memory execution wrapper

## Building

Fetch the vendored submodules once, then build and test with `cargo`:

```sh
just vendor   # or: git submodule update --init — only needed once
cargo test
```

There are [just](https://github.com/casey/just) recipes for the other operations (`just --list`).
