# Upgrade activation discards the old code's release log

When the old active code is provided, activating an upgrade unrequests it and
retains its referencer and balance charge until expunge. Rust emits
`ForgetAgainAt`; the pinned Quint pipeline loses that event.

In `quint/code_upgrades.qnt`, `activateUpgradeIfMatch` returns the release delta,
including its log. In `quint/accumulate.qnt`, `applyPreReplaySteps` uses the
activated state and forget output but returns `logs: List()`. Rust's
`service/src/accumulate/code_upgrades.rs` forwards the release log through
`release_code_if_not_pinned` into the package's logs. This is a model/Rust
discrepancy at model pin `4a22816d19a943688a3eef82b3fe4446667de812`;
no log normalization has been added to the replay comparator.

Reproduce using `service/bin/tests/fixtures/quint/upgrades.qnt`:

1. In `activation_works`, insert
   `.then(provide(vchAsHash(oldCode), FixedCodeLen))` immediately after `init`.
2. Replace the final old-key absence assertions with
   `svc.preimageStatus.get(oldKey) == Unrequested(now)` and
   `svc.preimageRegistry == prevSvc.preimageRegistry`.
3. Compare final `usedStateBalance` with `prevSvc` instead of `initialState`:
   the provided old code's charge is retained. Keep the unchanged-log assertion.
4. Run `python3 scripts/generate-quint-replays.py`, then
   `cargo test -p parachain-service-bin --test quint_replay upgrades::activation_works`.

Quint passes. Rust replay fails at activation (frame 9, slot 4):
`svc.parachainLog[1] differs; Quint=[]`, while Rust contains `ForgetAgainAt`
for the old code, length 65536, due 19204. All earlier transitions compare
successfully. The committed activation trace uses the initial unprovided old
code, whose release refunds immediately and produces no event in either system.

The model also drops the timeout-reap log in this pre-replay path; provided-code
expiry needs separate coverage. Resolve the expected logging behavior upstream
before extending replay to these cases, without changing the vendored pin alone.
