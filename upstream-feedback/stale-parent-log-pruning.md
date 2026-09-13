# Rejected-candidate log pruning: specification concern

The streaming replay input generator found this difference against Quint gitlink
`4a22816d19a943688a3eef82b3fe4446667de812`, using Quint 0.32.0's TypeScript simulator.
The trace uses the model's normal Refine and Accumulate functions, one WP per
block, valid lookup anchors, and no upward messages.

The original mismatch was found at Rust commit `59768be`. Rust now follows the
pinned specification and prunes on rejection. The proposed accepted-only pruning
rule is tracked in [issue #35](https://github.com/paritytech/parachain-service/issues/35).

Replay the original seeds from the repository root:

```sh
QUINT_FUZZ_SEED=1 QUINT_FUZZ_TRACES=1 QUINT_FUZZ_STEPS=30 \
  cargo test -p parachain-service-bin --test quint_replay \
  fuzz::generated_traces_works -- --ignored --nocapture
```

Before the Rust alignment, at frame 11 the generated candidate carried a stale parent and a lookup anchor
newer than earlier Coretime log entries. Quint cleared those entries; Rust kept
them. The comparator reported `frame 11: svc.parachainLog[1] differs; Quint=[]`.
Seed 2 independently found the same difference at frame 14. Complete traces and
errors are saved automatically under `target/quint-fuzz` for replay without Quint.

Relevant implementation paths:

- `vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate.qnt`,
  `accumulateOnePackage`: when `accumulateOkPrefix` returns `pre.rejected`, it
  nevertheless calls `pruneLogBelow(pre.state, ok.paraId, ok.lookupAnchor)`.
- `service/src/accumulate/package.rs`: lookup-anchor pruning now precedes the
  candidate rejection checks, matching the pinned model. Refine failures and
  JAM work errors still do not prune.

The concern remains: a rejected candidate does not advance the parachain, so
its handling of earlier logs cannot be committed in the canonical parachain state.
Preserving those logs would let a subsequent accepted candidate respond to them.
Until the specification changes, Rust follows its current behavior.

`log_pruning/stale_parent_seed_1_works.itf.json` preserves frames 0–11 of the
original saved seed-1 trace, without changing the model states. The ordinary
replay suite checks it without requiring Quint.
