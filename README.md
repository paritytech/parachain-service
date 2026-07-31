# Parachain Service PoC

The parachain service allows to run Polkadot parachains on JAM. It:

- Replaces the *candidate inclusion* with an implementation of JAM `accumulate`.
- Replaces the off-chain validation of *candidate backing* with an implementation of JAM `refine`.
- Offloads *approval checking*, *availability* and *backing* to JAM.
- Exposes a new Cumulus interface for collators to author parachain blocks.

There is on more relay chain runtime; the former relay chain logic is implemented without FRAME
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

There are [just](https://github.com/casey/just) commands for convenience operations but the main one
currently is:

```sh
git submodule update --init --recursive # Only needed once
cargo test
```
