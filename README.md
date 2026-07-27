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
├── pallets
│   ├── asset-hub-system
│   ├── coretime-system
│   └── parachain-system
├── runtimes
│   ├── asset-hub
│   └── coretime
├── service
```
