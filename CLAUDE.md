This implements the parachain service that will make Polkadot Parachains work on JAM.

## Resources

Specs and references (Quint, Cumulus, Gray Paper) are vendored under `vendor/` — see
[VENDOR.md](./VENDOR.md).

- Project Plan: https://hackmd.io/16r_PWiUQTuStKtZZx-0Bw.md (fetch manually)

## Spec Gaps

See [SPEC_GAPS.md](./SPEC_GAPS.md).

## Conventions

- Write test names in the form of `[<context>|trivial]_[errors|works]`. For example, `two_work_items_errors` or `trivial_works`. Assume that the file or module name is prefixed to the test function name.
- Put things into their own files, if it makes sense. This keeps merge conflicts minimal and allows for easier navigation. For example: `refine.rs`, `accumulate.rs`, `is_authorized.rs`, etc.
- Use `TODO:` for uncritical stuff that can be done later and `FIXME:` for consensus critical things that needs to be fixed before production usage.
