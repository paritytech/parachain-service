`refine_errors.qnt` generates the 29 fixtures in `refine_errors/` for the
Accumulate regressions in `tests/quint_replay/refine_errors.rs`. It imports the
model from the pinned `vendor/polkadot-sdk-quint` submodule. Expected states come
from the model's `accumulateBlock`; these scenarios supply work results and do
not run Rust Refine.

Coverage within the existing replay adapter:

- All seven supported refine-error variants, including `InvalidCodeHash`.
- Each variant targeting an unregistered parachain (no log or registration).
- Every nullary variant with auth-trace lengths 0, 255, 256, and 257; traces
  above 256 bytes must be truncated in storage.
- Opaque payload lengths 0, 42, 63, 64, 1023, and 1024, including the SCALE
  compact-length boundary and maximum payload, with varied auth-trace lengths.
- Repeated and mixed errors append without pruning earlier entries.
- An error for an unregistered parachain preserves another parachain's log.
- JAM work errors preserve existing logs and leave an empty log empty.

The current model/adapter cannot separately replay Rust's
`SetValidatorKeysRepeated`, `UpwardMessagesTooLarge`, `InvalidAuthorizerQueue`,
`MalformedPayload`, or `HeadDataTooLarge`. The model folds repeated validator
keys into `SetValidatorKeysTooManyKeys`; the adapter maps that to Rust's
`TooManyValidatorKeys`. Model authorizer/config and item-count failures are
intentionally rejected by the adapter: Rust rejects authorization or fails the
whole work item, rather than emitting those as logged refine errors.

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

All ITF fixtures are committed as compact JSON, and the generator writes that
format directly. To expand them in place for review, run `just quint-fmt`.
Run `just quint-compact` before committing to restore compact JSON. Both commands
require Python 3 and preserve the parsed JSON values.
