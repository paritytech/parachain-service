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
`refine_errors`, `blocks`, `upgrades`, `log_pruning`, `kv`, `balances`, `lifecycle`, and `assignments` scenarios, including the
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
| Lifecycle | Registration thresholds and repeated funding, unauthorized calls, forced head/code changes, cleanup refusal with extra storage, delayed cleanup, and re-registration |
| Assignments | Immediate/delayed execution, due-slot boundaries, queue expansion/rotation, repeated replacements, authorization, invalid queues, handoffs, a cached assign rejected after a handoff, pending storage, and final JAM queues/privileges |
| Upgrades | Announcements and applies, supersession, refused forgets of validation code, unavailable or foreign code, failed reservations, and skipped work |
| Log pruning | Rejected candidates retain logs; accepted candidates prune below the lookup anchor and retain the boundary |

Mutation tests check that storage, log, and commitment mismatches are rejected.

## Adapter limits

- Validator-key inputs are unsupported; blocks must expect no staging-set change.
  Unexpected designation, transfer, provide, create, or eject effects fail.
- Assignment messages and initial pending queues are supported. Authorizer integers
  map to a u32 little-endian prefix padded to 32 bytes. Assignment service IDs swap
  Quint 1 with mock 0; all other IDs remain literal (incoming-transfer IDs retain
  their existing literal mapping). Replay starts with this service owning every
  core and carries actual JAM privileges between blocks, including handoffs.
  Quint's ordered `lastStepAssigns` is folded into expected final queues and
  privileges, checking all privilege fields and rejecting extra assigned cores.
  The vendored host exposes only final mutations, so overwritten intermediate
  calls and ordering between independent cores cannot be compared. The model's
  `jamCoreAssigners` ghost state is checked only through those privileges.
- Incoming-transfer replay requires explicit `replayIncoming` operands and an
  initially empty queue. Regular-balance arrivals are supported; supervisor
  arrivals fail explicitly because the vendored host has no selector. Integer
  memos use a u64 little-endian prefix padded to 128 bytes. The model does not
  represent the JAM service's actual monetary balance.
- Accumulate-log decoding supports `ForgetAgainAt`, `StateBalanceUpdateRejected`,
  `TooMuchStateHeld`, `InvalidCodeHashAcc`, `CodeUpgradeNotAvailable`,
  `CodeUpgradeNotAnnounced`, `CanNotForgetValidationCode`, `CoreNotAssignable`, and
  `InsufficientStateBalance` from `FromSolicit` or `FromSetKV`; other events fail explicitly.
- `Solicit` and `Forget` support explicit parachain targets, including delegated
  calls, and historical fixtures without `Target`. Service targets are rejected
  pending host support ([DIVERGENCE.md M-11](../../../../../DIVERGENCE.md#m-11-the-model-decides-the-65-supervised-service-outcomes-rust-can-only-refuse)).
- Abstract hashes require a consistent preimage length within each domain.
  `solicitedSet` is model ghost state, not a returned JAM output.
- KV keys and values are literal byte lists. Failure-log key hashes use a separate
  codex for Quint's base-257 `listHash`; ambiguous hashes (such as empty and
  leading-zero keys) and hashes exceeding i128 fail explicitly. The generator
  uses a small collision-free key pool.
