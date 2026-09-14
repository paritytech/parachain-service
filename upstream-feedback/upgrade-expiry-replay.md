# Upgrade expiry differences exposed by replay

At Quint pin `4a22816d19a943688a3eef82b3fe4446667de812`, the expanded generator
samples `RequestCodeUpgrade`, independent solicit/forget of upgrade code, and
pending-code candidates. No Rust service behavior or vendored model was changed.

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

## An expired-code candidate mutates only the model's state

Request code 777, provide it, cross its deadline, then submit a candidate validated
with 777. Quint reaps the upgrade before checking the candidate's code, rejects
the candidate, and persists the reaped state. Rust checks code eligibility using
the post-expiry view before making those writes, so rejection preserves the
pending upgrade. Quint also records `InvalidCodeHashAcc`, which has no Rust
Accumulate-log counterpart; replay first reports the pending-state disagreement.

The minimal fixture is
`service/bin/tests/fixtures/quint/upgrades/expired_code_candidate_reaps_works.itf.json`.
The ordinary `upgrades::expired_code_candidate_reaps_errors` test asserts the
pending-upgrade disagreement. The generated campaign first reached this at seed 2,
frame 21, with 50 steps.

## Reproduction

Regenerate these fixtures with Quint 0.32.0:

```sh
python3 -c 'import runpy; runpy.run_path("scripts/generate-quint-replays.py")["generate"]("upgrades")'
cargo test -p parachain-service-bin --test quint_replay upgrades::
```

Those ordinary tests pass by detecting the known disagreements. To observe a
raw differential failure, replay either fixture through the strict input runner:

```sh
QUINT_REPLAY_TRACE=service/bin/tests/fixtures/quint/upgrades/expired_code_candidate_reaps_works.itf.json cargo test -p parachain-service-bin --test quint_replay fuzz::replay_input_works -- --ignored --nocapture
```

The full campaign also remains strict and saves failing traces:

```sh
QUINT_FUZZ_TRACES=100 QUINT_FUZZ_STEPS=50 QUINT_FUZZ_WORKERS=2 cargo test -p parachain-service-bin --test quint_replay fuzz::generated_traces_works -- --ignored --nocapture
```

Resolve the intended service/model behavior separately. Do not filter these
inputs or normalize away the differences to make the campaign pass.
