# Stale-parent candidates prune logs in Quint but preserve them in Rust

The streaming replay input generator found this difference against Quint gitlink
`4a22816d19a943688a3eef82b3fe4446667de812`, using Quint 0.32.0's TypeScript simulator.
The trace uses the model's normal Refine and Accumulate functions, one WP per
block, valid lookup anchors, and no upward messages.

Reproduce from the repository root:

```sh
QUINT_FUZZ_SEED=1 QUINT_FUZZ_TRACES=1 QUINT_FUZZ_STEPS=30 \
  cargo test -p parachain-service-bin --test quint_replay \
  fuzz::generated_traces_works -- --ignored --nocapture
```

At frame 11, the generated candidate carries a stale parent and a lookup anchor
newer than earlier Coretime log entries. Quint clears those entries; Rust keeps
them. The comparator reports `frame 11: svc.parachainLog[1] differs; Quint=[]`.
Seed 2 independently finds the same difference at frame 14. Complete traces and
errors are saved automatically under `target/quint-fuzz` for replay without Quint.

Relevant implementation paths:

- `vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate.qnt`,
  `accumulateOnePackage`: when `accumulateOkPrefix` returns `pre.rejected`, it
  nevertheless calls `pruneLogBelow(pre.state, ok.paraId, ok.lookupAnchor)`.
- `service/src/accumulate/package.rs`: a parent-head mismatch returns before
  `ParachainLogs::prune_below`. The comment explains the intent: rejected
  candidates should not be able to erase logs via their chosen lookup anchors.

This is recorded for separate resolution. The streaming machinery does not
normalize away the difference, exclude stale parents, or change either behavior.
