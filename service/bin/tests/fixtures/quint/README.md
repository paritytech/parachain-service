# Quint replay fixtures

These fixtures replay recorded work results through Rust Accumulate in the PVM
and compare storage and returned head commitments. Rust Refine is not executed.
For streaming campaigns and saved-failure replay, see [FUZZING.md](FUZZING.md).

## Regenerate and replay

From the repository root, with Python 3 and Quint **0.32.0** on PATH:

```sh
python3 scripts/generate-quint-replays.py
cargo test -p parachain-service-bin --test quint_replay
```

The generator uses the pinned model, TypeScript backend, and seed 1. It regenerates
`refine_errors`, `blocks`, `upgrades`, and `log_pruning` scenarios, including the
root minimal and stale-parent fixtures. Other historical fixtures are retained.
JSON is compact and timestamp-free; use `just quint-fmt` to expand it for review
and `just quint-compact` before committing.

## Coverage

| Scenarios | Checks |
| --- | --- |
| Refine errors, WorkErr, empty blocks | Error logs and unchanged state for skipped work |
| Multiple work packages and parachains | Ordered processing, shared references, delegated forgets, and combined head commitments |
| Upgrades | Requests, provision, activation, expiry, failed reservations, and rejected expired-code candidates |
| Log pruning | Rejected candidates retain logs; accepted candidates prune below the lookup anchor and retain the boundary |

Mutation tests check that storage, log, and commitment mismatches are rejected.
Provided-code activation and expiry currently omit `ForgetAgainAt` to match Quint;
[issue #36](https://github.com/paritytech/parachain-service/issues/36) tracks the model change.

## Adapter limits

- Incoming transfers, assignment inputs, and validator-key inputs are unsupported.
  Blocks must expect empty assignments and no staging-set change. Unexpected Rust
  assignment, privilege, designation, transfer, provide, create, or eject effects fail.
- Accumulate-log decoding supports `ForgetAgainAt`, `StateBalanceUpdateRejected`,
  and `InsufficientStateBalance(FromSolicit)`; other events fail explicitly.
- `Solicit` and `Forget` support explicit parachain targets, including delegated
  calls, and historical fixtures without `Target`. Service targets are rejected
  pending host support ([DIVERGENCE.md M-11](../../../../../DIVERGENCE.md#m-11-the-model-decides-the-65-supervised-service-outcomes-rust-can-only-refuse)).
- Abstract hashes require a consistent preimage length within each domain.
  `solicitedSet` is model ghost state, not a returned JAM output.
