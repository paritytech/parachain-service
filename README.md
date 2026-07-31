# Parachain Service PoC

The parachain service lets you run Polkadot parachains on JAM. It:

- Replaces the *candidate inclusion* with an implementation of JAM `accumulate`.
- Replaces the off-chain validation of *candidate backing* with an implementation of JAM `refine`.
- Offloads *approval checking*, *availability* and *backing* to JAM.
- Exposes a new Cumulus interface for collators to author parachain blocks.

There is no longer a relay chain runtime; the former relay chain logic is implemented without FRAME
directly as an `accumulate` hook. Parachain runtimes need to use a new parachain system pallet and
for Asset Hub and Coretime there are special system pallets.

## Project Structure

```
.
├── authorizer          # JAM authorizer for the service
├── pallets             # System pallets wired into the runtimes
│   ├── asset-hub-system
│   └── coretime-system
├── runtimes            # Parachain runtimes
│   ├── asset-hub
│   └── coretime
├── scripts             # Justfile modules
├── service             # The parachain service (refine + accumulate)
└── tools
    ├── executor        # PVM/runtime executor library
    └── executor-cli    # Debug CLI around the executor
```

## Building

Fetch the vendored submodules once, then build and test with `cargo`:

```sh
just vendor   # or: git submodule update --init — only needed once
cargo test
```

There are [just](https://github.com/casey/just) recipes for the other operations (`just --list`).
