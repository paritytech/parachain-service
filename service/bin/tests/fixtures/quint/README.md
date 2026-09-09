`refine_errors.qnt` generates the ten fixtures in `refine_errors/` for the
Accumulate regressions in `tests/quint_replay/refine_errors.rs`. It imports the
model from the pinned `vendor/polkadot-sdk-quint` submodule. Each trace has three
frames: initialization, a supplied refine-error work result, and a JAM work error
that must preserve the log. Expected states come from the model's
`accumulateBlock`; these scenarios do not run Rust Refine.

From the repository root, with Python 3 and Quint 0.32.0 installed:

```sh
python3 scripts/generate-refine-error-replays.py
cargo test -p parachain-service-bin --test quint_replay
```

Generation uses the TypeScript backend and seed 1. The script removes only ITF
metadata (timestamps and frame indexes) and sorts JSON keys and the variable list
for deterministic output compatible with the Rust ITF decoder. All generated
state values are preserved. The three negative Rust tests deliberately corrupt a
generated fixture's expected log to verify replay mismatch detection.

The other fixtures in this directory predate this generator and are not rewritten
by it.
