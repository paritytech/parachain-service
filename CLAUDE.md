This implements the parachain service that will make Polkadot Parachains work on JAM.

## Resources

Specs and references (Quint, Cumulus, Gray Paper) are vendored as git submodules under `vendor/`; the
pinned revisions are the gitlinks themselves. The Parachain Service design and Quint spec live in
`vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/`, and its pin tracks the tip of
`bkchr-parachain-service-doc` ([PR #11883](https://github.com/paritytech/polkadot-sdk/pull/11883)),
moved forward one upstream commit at a time with the matching Rust change in the same commit.

- Project Plan: https://hackmd.io/16r_PWiUQTuStKtZZx-0Bw.md (fetch manually)

## Conventions

- Write test names in the form of `[<context>|trivial]_[errors|works]`. For example, `two_work_items_errors` or `trivial_works`. Assume that the file or module name is prefixed to the test function name.
- Put things into their own files, if it makes sense. This keeps merge conflicts minimal and allows for easier navigation. For example: `refine.rs`, `accumulate.rs`, `is_authorized.rs`, etc.
- Use `TODO:` for uncritical stuff that can be done later and `FIXME:` for consensus critical things that needs to be fixed before production usage.
- Be aware that there are two identically named `Encode` etc traits. One from SCALE `codec` crate and one from `jam_codec`. Dont mix them up.
