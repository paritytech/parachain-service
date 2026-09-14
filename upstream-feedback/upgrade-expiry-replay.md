# Upgrade expiry differences exposed by replay

At Quint pin `4a22816d19a943688a3eef82b3fe4446667de812`, the expanded generator
samples `RequestCodeUpgrade`, independent solicit/forget of upgrade code, and
pending-code candidates. The vendored model remains unchanged; Rust now reaps
expired upgrades before rejecting candidates, as required by §5.1 steps 4–5.

## Provided-code expiry loses a log

Request code 777, provide it, cross the upgrade deadline, then submit an active-code
candidate. Both implementations clear the upgrade and retain the provided
preimage's reference while it awaits expunge. Rust logs `ForgetAgainAt`; Quint
returns an empty log. `accumulateOkPrefix` drops the log from
`reapTimedOutUpgrade`, just as it drops activation release logs described in
[upgrade-activation-log.md](upgrade-activation-log.md).

The minimal fixture is
`service/bin/tests/fixtures/quint/upgrades/provided_upgrade_expiry_log_works.itf.json`.
The ordinary `upgrades::provided_upgrade_expiry_log_errors` test asserts the
specific log disagreement. The generated campaign first reached this at seed 1,
frame 32, with 50 steps.

## Expired-code rejection now reaps in Rust; the model still loses a log

Request code 777, provide it, cross its deadline, then submit a candidate validated
with 777. Both implementations now reap the upgrade before checking the code,
reject the candidate, and persist the reaped state. Rust records the cleanup's
`ForgetAgainAt` followed by `InvalidCodeHash`; Quint records only
`InvalidCodeHashAcc`, dropping the cleanup log in `accumulateOkPrefix`.

Previously Rust returned before applying the reap and omitted the invalid-code
log. Seed 2709851819 exposed that state mismatch at frame 11: the upgrade to 778
was requested at slot 14402 with deadline 28802, then used by a candidate at slot
43207. With the Rust fix, the saved trace reaches log comparison at that frame;
`pendingUpgrade` and the preimage cleanup agree.

The minimal fixture is
`service/bin/tests/fixtures/quint/upgrades/expired_code_candidate_reaps_works.itf.json`.
The ordinary `upgrades::expired_code_candidate_reap_log_errors` test asserts the
remaining log disagreement. Direct Rust tests in `accumulate_upgrades.rs` check
provided and unprovided expiry, cleanup logs/refunds, stale-parent rejection, and
that rejected candidates do not enact their heads or upward messages.

## Reproduction

Regenerate these fixtures with Quint 0.32.0:

```sh
python3 -c 'import runpy; runpy.run_path("scripts/generate-quint-replays.py")["generate"]("upgrades")'
cargo test -p parachain-service-bin --test quint_replay upgrades::
```

Those ordinary tests pass by detecting the known disagreements. To observe a
raw differential failure, replay either fixture through the strict input runner:

```sh
QUINT_REPLAY_TRACE="$PWD/service/bin/tests/fixtures/quint/upgrades/expired_code_candidate_reaps_works.itf.json" cargo test -p parachain-service-bin --test quint_replay fuzz::replay_input_works -- --ignored --nocapture
```

The full campaign also remains strict and saves failing traces:

```sh
QUINT_FUZZ_TRACES=100 QUINT_FUZZ_STEPS=50 QUINT_FUZZ_WORKERS=2 cargo test -p parachain-service-bin --test quint_replay fuzz::generated_traces_works -- --ignored --nocapture
```

The remaining model fix is to retain expiry cleanup logs on acceptance and
rejection. Do not filter these inputs or normalize away the differences to make
the campaign pass.
