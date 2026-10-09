This implements the parachain service that will make Polkadot Parachains work on JAM.

## Resources

Specs and references (Quint, Cumulus, Gray Paper) are vendored as git submodules under `vendor/`; the
pinned revisions are the gitlinks themselves. The Parachain Service design and Quint spec live in
`vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/`, and its pin tracks the tip of
`bkchr-parachain-service-doc` ([PR #11883](https://github.com/paritytech/polkadot-sdk/pull/11883)),
moved forward one upstream commit at a time with the matching Rust change in the same commit.

- Project Plan: https://hackmd.io/16r_PWiUQTuStKtZZx-0Bw.md (fetch manually)

## Conventions

- After every code change, run the full workspace test suite, including gas tests and opt-in replay/generated tests with their required fixtures and environment. Do not substitute targeted tests for the full suite, ignore failing tests, or report success while tests fail or remain unrun. Fix failures and report any genuine external blocker explicitly.

- Follow [ISSUE_GUIDELINES.md](ISSUE_GUIDELINES.md) when writing or editing issues.
- Write test names in the form of `[<context>|trivial]_[errors|works]`. For example, `two_work_items_errors` or `trivial_works`. Assume that the file or module name is prefixed to the test function name.
- Put things into their own files, if it makes sense. This keeps merge conflicts minimal and allows for easier navigation. For example: `refine.rs`, `accumulate.rs`, `is_authorized.rs`, etc.
- Use `TODO:` for uncritical stuff that can be done later and `FIXME:` for consensus critical things that needs to be fixed before production usage.
- Be aware that there are two identically named `Encode` etc traits. One from SCALE `codec` crate and one from `jam_codec`. Dont mix them up.

## Full test run

Use `--profile testnet` for the full test suite and fuzzing. It enables LTO and a single
codegen unit while retaining debug assertions and overflow checks. Plain `cargo test` uses
the debug test profile and is also supported. Guest blobs use their separately configured
production profiles, which determine the pinned PVM gas measurements.

Run from the repository root. This includes the opt-in replay/generated tests and doc tests:

```sh
QUINT_REPLAY_TRACE="$PWD/service/bin/tests/fixtures/quint/minimal_replay.itf.json" \
QUINT_FUZZ_PROFILE=fuzz QUINT_FUZZ_SEED=1 QUINT_FUZZ_TRACES=100 \
QUINT_FUZZ_STEPS=15 QUINT_FUZZ_WORKERS=4 \
cargo test --locked --profile testnet --workspace --no-fail-fast -- \
  --include-ignored --test-threads=4
```
