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
`refine_errors`, `blocks`, `upgrades`, `log_pruning`, `kv`, and `balances` scenarios, including the
root minimal and stale-parent fixtures. Other historical fixtures are retained.
JSON is compact and timestamp-free; use `just quint-fmt` to expand it for review
and `just quint-compact` before committing.

## Coverage

| Scenarios | Checks |
| --- | --- |
| Refine errors, WorkErr, empty blocks | Error logs and unchanged state for skipped work |
| Multiple work packages and parachains | Ordered processing, shared references, delegated forgets, and combined head commitments |
| KV operations | Overwrite, empty values/keys, SCALE length boundary, refunds, delegated and unauthorized removal, failed reservations, and stale candidates |
| Balances and incoming transfers | Allowance boundaries, authorization, reservations/refunds, queue packing and rollover, admission/drop at the reservation limit, and Asset Hub charges |
| Upgrades | Requests, provision, activation, expiry, failed reservations, and rejected expired-code candidates |
| Log pruning | Rejected candidates retain logs; accepted candidates prune below the lookup anchor and retain the boundary |

Mutation tests check that storage, log, and commitment mismatches are rejected.
Provided-code activation and expiry currently omit `ForgetAgainAt` to match Quint;
[issue #36](https://github.com/paritytech/parachain-service/issues/36) tracks the model change.

## Adapter limits

- Assignment inputs and validator-key inputs are unsupported.
  Blocks must expect empty assignments and no staging-set change. Unexpected Rust
  assignment, privilege, designation, transfer, provide, create, or eject effects fail.
- Incoming-transfer replay requires explicit `replayIncoming` operands and an
  initially empty queue. Regular-balance arrivals are supported; supervisor
  arrivals fail explicitly because the vendored host has no selector. Integer
  memos use a u64 little-endian prefix padded to 128 bytes. The model does not
  represent the JAM service's actual monetary balance.
- Accumulate-log decoding supports `ForgetAgainAt`, `StateBalanceUpdateRejected`,
  `InvalidCodeHashAcc`, and `InsufficientStateBalance` from `FromSolicit` or
  `FromSetKV`; other events fail explicitly.
- `Solicit` and `Forget` support explicit parachain targets, including delegated
  calls, and historical fixtures without `Target`. Service targets are rejected
  pending host support ([DIVERGENCE.md M-11](../../../../../DIVERGENCE.md#m-11-the-model-decides-the-65-supervised-service-outcomes-rust-can-only-refuse)).
- Abstract hashes require a consistent preimage length within each domain.
  `solicitedSet` is model ghost state, not a returned JAM output.
- KV keys and values are literal byte lists. Failure-log key hashes use a separate
  codex for Quint's base-257 `listHash`; ambiguous hashes (such as empty and
  leading-zero keys) and hashes exceeding i128 fail explicitly. The generator
  uses a small collision-free key pool.
