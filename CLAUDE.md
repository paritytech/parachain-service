This implements the parachain service that will make Polkadot Parachains work on JAM.

## Resources

Ensure that you have access to all the resources below. Check the `.env` file for paths. They can
either already be checked out on machine at something like `../` or need to be newly cloned into the
gitignored `vendor/` directory. Ask the user whether it should be freshly cloned or not. Once you
have done this, write the paths to the resources in the `.env` file for future reference. Use `gh`,
if available.

- WIP Dafny spec: https://github.com/paritytech/polkadot-sdk/pull/11883
- WIP Cumulus Spec: https://github.com/paritytech/polkadot-sdk/blob/mku-cumulus-on-jam-doc/designs/parachain-service-on-jam/parachain-service-on-jam.md
- JAM Gray Paper: https://github.com/gavofyork/graypaper
- Project Plan: https://hackmd.io/16r_PWiUQTuStKtZZx-0Bw.md (needs to be fetched by the user manually)

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

## Conventions

- Write test names in the form of `[<context>|trivial]_[errors|works]`. For example, `two_work_items_errors` or `trivial_works`. Assume that the file or module name is prefixed to the test function name.
- Put things into their own files, if it makes sense. This keeps merge conflicts minimal and allows for easier navigation. For example: `refine.rs`, `accumulate.rs`, `is_authorized.rs`, etc.
- Use `TODO:` for uncritical stuff that can be done later and `FIXME:` for consensus critical things that needs to be fixed before production usage.
