# Upgrade expiry differences exposed by replay

Quint `06c2a49202` changed rejection to discard tentative expiry cleanup. Rust
now matches that rule. Rust also omits the accepted-candidate expiry log to match
Quint while
[issue #36](https://github.com/paritytech/parachain-service/issues/36) is unresolved.

## Provided-code expiry loses a log

Request code 777, provide it, cross the upgrade deadline, then submit an active-code
candidate. Both implementations clear the upgrade and retain the provided
preimage's reference while it awaits expunge. Quint returns an empty log. Rust now
matches this, discarding the `ForgetAgainAt` returned by cleanup at the package
boundary. `accumulateOkPrefix` drops the log from
`reapTimedOutUpgrade`, just as it drops activation release logs described in
[upgrade-activation-log.md](upgrade-activation-log.md).

The minimal fixture is
`service/bin/tests/fixtures/quint/upgrades/provided_upgrade_expiry_log_works.itf.json`.
The ordinary `upgrades::provided_upgrade_expiry_log_works` test asserts strict
agreement, including the empty log. The generated campaign first reached this at
seed 1, frame 32, with 50 steps.

## Expired-code rejection: resolved

An expired-code candidate cannot match the pending code in the post-expiry view,
so it is rejected without committing the reap, forwarding a forget, logging an
invalid-code error, or pruning. The regenerated
`upgrades/expired_code_candidate_preserves_upgrade_works.itf.json` now passes
strict replay. Direct Rust tests cover both provided and unprovided pending code.

## Reproduction

Regenerate these fixtures with Quint 0.32.0:

```sh
python3 -c 'import runpy; runpy.run_path("scripts/generate-quint-replays.py")["generate"]("upgrades")'
cargo test -p parachain-service-bin --test quint_replay upgrades::
```

The provided-code expiry test now passes. Replay that fixture through the strict
input runner:

```sh
QUINT_REPLAY_TRACE="$PWD/service/bin/tests/fixtures/quint/upgrades/provided_upgrade_expiry_log_works.itf.json" cargo test -p parachain-service-bin --test quint_replay fuzz::replay_input_works -- --ignored --nocapture
```

The full campaign also remains strict and saves failing traces:

```sh
QUINT_FUZZ_TRACES=100 QUINT_FUZZ_STEPS=50 QUINT_FUZZ_WORKERS=2 cargo test -p parachain-service-bin --test quint_replay fuzz::generated_traces_works -- --ignored --nocapture
```

The proposed model fix is to retain expiry cleanup logs on acceptance. Once #36
is resolved, revisit the Rust compatibility behavior and regenerate the fixture
with the matching upstream pin. The replay comparator remains strict.
