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
`refine_errors`, `blocks`, `upgrades`, `log_pruning`, `kv`, `balances`, `lifecycle`, `assignments`, and `validator_keys` scenarios, including the
root minimal and stale-parent fixtures. Other historical fixtures are retained.
JSON is compact and timestamp-free; use `just quint-fmt` to expand it for review
and `just quint-compact` before committing.

## Coverage

| Scenarios | Checks |
| --- | --- |
| Refine errors, WorkErr, empty blocks | Error logs, unlogged failures of other code, and unchanged state for skipped work |
| Multiple work packages and parachains | Ordered processing, shared references, delegated forgets, and combined head commitments |
| KV operations | Overwrite, empty keys/values rejected at Refine, values crossing 64 bytes without a length prefix, refunds, delegated and unauthorized removal, failed reservations, and stale candidates |
| Balances and incoming transfers | Allowance boundaries, authorization, reservations/refunds, queue packing and rollover, admission/drop at the reservation limit, and Asset Hub charges |
| Lifecycle | Registration thresholds and repeated funding, unauthorized calls, forced head/code changes, cleanup refusal with extra storage, delayed cleanup, and re-registration |
| Assignments | Immediate/delayed execution, due-slot boundaries, queue expansion/rotation, repeated replacements, authorization, invalid queues, handoffs, a cached assign rejected after a handoff, pending storage, and final JAM queues/privileges |
| Validator keys | Chunk staging, seeded buffers, full-size designation, same-set re-designation, empty aborts, invalid lengths, overflow, authorization, stale work, and final designation across multiple work packages |
| Upgrades | Announcements and applies, supersession, refused forgets of validation code, unavailable or foreign code, failed reservations, and skipped work |
| Log pruning | Rejected candidates retain logs; accepted candidates prune below the lookup anchor and retain the boundary |

Mutation tests check that storage, log, commitment, and designation mismatches are rejected.

## Adapter limits

- Validator-key integers map to a u64 little-endian prefix padded to 336 bytes.
  Initial staging buffers are seeded, and stored buffers and final JAM key sets
  are compared byte-for-byte in order. Blocks containing `SetValidatorKeys` must
  include `replayDesignate`: the pinned model's last successful designation, or
  an empty list for none. This distinguishes no call from designation of an
  unchanged set. Historical fixtures without key inputs may omit this field.
  The fixture wrapper evaluates `accumulateBlock` with an empty incoming JAM set
  to expose that effect, then carries the prior set forward when no call occurs.
  The host exposes only the final successful designation, so intermediate calls
  overwritten in a block remain unobservable. The model assumes this service
  holds the designation privilege; unprivileged-host rejection remains covered
  by the direct Rust tests. The fuzz input pool includes key updates and records the same effect.
  Unexpected transfer, provide, create, or eject effects fail.
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
  memos use a u64 little-endian prefix padded to 128 bytes. The model's JAM
  balances (`svc.jamAccount`, and each foreign service's `account`) are ghost
  state and are not compared.
- Accumulate-log decoding supports `ForgetAgainAt`, `StateBalanceUpdateRejected`,
  `TooMuchStateHeld`, `InvalidCodeHashAcc`, `CodeUpgradeNotAvailable`,
  `CodeUpgradeNotAnnounced`, `CanNotRemoveCode`, `CoreNotAssignable`,
  `DesignateRejected`, `StagedValidatorKeysOverflow`, and
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
