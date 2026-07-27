This implements the parachain service that will make Polkadot Parachains work on JAM.

## Resources

Ensure that you have access to all the resources below. Check the `.env` file for paths. They can
either already be checked out on machine at something like `../` or need to be newly cloned into the
gitignored `vendor/` directory. Once you have done this, write the paths to the resources in the
`.env` file for future reference.

- WIP Dafny spec: https://github.com/paritytech/polkadot-sdk/pull/11883
- WIP Cumulus Spec: https://github.com/paritytech/polkadot-sdk/blob/mku-cumulus-on-jam-doc/designs/parachain-service-on-jam/parachain-service-on-jam.md
- 0.8.0 JAM Gray Paper: https://github.com/gavofyork/graypaper

## Objectives

1. Implement PoC Parachain Service with both Refine and Accumulate entry points.
2. Improve spec with discovered issues and sharp edges from 1.
3. Repeat 1-2 until spec is in acceptable shape.

## Spec Gaps

See [SPEC_GAPS.md](./SPEC_GAPS.md).

## Build Instructions

To build the PVM service blob in `target/parachain-service.jam`, run:

```bash
just build
```
