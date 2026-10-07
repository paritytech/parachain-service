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
`refine_errors`, `blocks`, `upgrades`, `log_pruning`, `kv`, `balances`, `lifecycle`,
`assignments`, `validator_keys`, `outgoing`, `service_upgrades`, `mixed`, `gas`,
`storage`, `host_mixed`, `services`, `service_preimages`, `service_management`, and `panic_recovery` scenarios,
including the root minimal and stale-parent fixtures. Other historical fixtures
are retained.
JSON is compact and timestamp-free; use `just quint-fmt` to expand it for review
and `just quint-compact` before committing.

## Coverage

| Scenarios | Checks |
| --- | --- |
| Refine errors, WorkErr, empty blocks | Error logs, unlogged failures of other code, and unchanged state for skipped work |
| Pre-checkpoint gas recovery | Independent rollback oracle, retained incoming funds, reverted due assignments and queue writes, repeated exhaustion, and successful retries |
| Instruction gas recovery | Sampled invocation limits and cutoffs before/after writes, creation, and transfers; fixed Quint prefix, checkpoint effects, and subsequent invocations |
| Accumulate panic recovery | Actual PVM traps from malformed work digests; first/middle/last checkpoints, zero report gas, repeated faults, due assignments, retained host effects, and successful later invocations |
| Multiple work packages and parachains | Ordered processing, shared references, delegated forgets, and combined head commitments |
| KV operations | Overwrite, empty keys/values rejected at Refine, values crossing 64 bytes without a length prefix, refunds, delegated and unauthorized removal, failed reservations, and stale candidates |
| Mixed invocations | Arrivals and accepted/rejected reports in one call, bucket rollover, due assignments, and incoming credit funding an outgoing payment |
| Balances and incoming transfers | Allowance boundaries, authorization, reservations/refunds, queue packing and rollover, admission/drop at the reservation limit, and Asset Hub charges |
| Outgoing transfers | Ordered deferred records (source, destination, amount, memo, gas), zero amounts, repeated IDs, all refusal reasons, gas boundaries, rejected work, persistent debits, and service balances |
| Lifecycle | Registration thresholds and repeated funding, unauthorized calls, forced head/code changes, cleanup refusal with extra storage, delayed cleanup, and re-registration |
| Assignments | Immediate/delayed execution, due-slot boundaries, queue expansion/rotation, repeated replacements, authorization, invalid queues, handoffs, a cached assign rejected after a handoff, pending storage, and final JAM queues/privileges |
| Validator keys | Chunk staging, seeded buffers, full-size designation, same-set re-designation, empty aborts, invalid lengths, overflow, authorization, stale work, and final designation across multiple work packages |
| Upgrades | Announcements and applies, supersession, refused forgets of validation code, unavailable or foreign code, failed reservations, and skipped work |
| Service self-upgrades | Availability and Asset Hub reference checks, wrong lengths, unrequested/rerequested code, running-code protection, repeated upgrades and gas changes, ordered requests, and execution of installed code on later blocks |
| Service management | Storage removal and supervisor handoff refusals; target/new-supervisor error precedence, self and set-free cases, created accounts, authorization, stale work, gas gates, checkpoints, and host budgets |
| Service preimages | Solicit/forget refusals for unknown, seeded, created, and self targets; retained creation requests; authorization, stale work, gas gates, checkpoint recovery, and mixed host budgets |
| Log pruning | Rejected candidates retain logs; accepted candidates prune below the lookup anchor and retain the boundary |

Mutation tests check that storage, logs, commitments, designation, transfer records,
JAM balances, installed code lengths, and upgrade gas-setting mismatches are rejected.

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
  Unexpected provide or eject effects fail; creation and transfers use the explicit oracles below.
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
  initially empty queue. Frames may carry arrivals and work reports together;
  replay credits arrivals before one invocation containing both input types. Regular-balance arrivals are supported; supervisor
  arrivals fail explicitly because the vendored host has no selector. Integer
  memos use a u64 little-endian prefix padded to 128 bytes. The model's JAM
  balances (`svc.jamAccount`, and each foreign service's `account`) are compared
  only in the outgoing-transfer profile below.
- `CreateService` and `EjectService` run in the default fuzz campaign, including
  mixed arrivals, gas gates, and checkpoint recovery. Creation traces carry
  `replayCreations` in every frame, recording the allocated ID and original args
  for each completed creation. Replay compares the complete host creation set,
  balances, code hash, gas minima, parent, creation slot, footprint, and initial
  unprovided code request; later frames keep checking those accounts.
  `service_inputs.qnt` adapts only the pinned host differences: mock registrar
  allocation (public IDs start at 65536 per invocation, advancing by 42), ignored
  public desired IDs, unsupported balance selectors, and absent supervision.
  New accounts are self-supervised in the compatibility model; JAM records the
  creator as parent. Ejection therefore checks `TargetIsSelf`, `UnknownService`,
  and `NotSupervised`, including accounts created in the same or an earlier
  invocation. Successful ejection is unavailable on this host.
  The pinned host debits creation funding **before** returning `IdTaken` for an
  occupied protected ID. The compatibility oracle explicitly checks that debit;
  this is host behavior, not the design's atomic refusal semantics. The gitlink
  and production code remain unchanged.
- Outgoing-transfer traces carry `replayTransfers` in **every** frame, including
  initialization. `outgoing_inputs.qnt` observes accepted packages and successful
  deferred calls through the pinned model's prefix, message, transfer, and package
  functions; it does not infer success from net balances or unique failure IDs.
  Rust compares the ordered JAM records, then credits destinations from those
  actual records to represent the scheduler's later delivery. Destination code
  is not executed. Incoming operands credit the service before accumulation.
  Initial regular balances and minimum memo gas come from the model; both are
  compared after each transition. Foreign accounts must initially be empty and
  self-supervised. Service IDs swap model 1 with mock 0, as for assignments;
  incoming source IDs retain their historical literal mapping. Memos use a u64
  little-endian prefix padded to 128 bytes.
  The generated domain covers small deferred payments to foreign services,
  zero payments (including an empty source supervisor balance), definite
  overdrafts, and host-compatible refusals. It excludes successful supervision
  operations and nonzero supervisor credits. A separate `self_payment` profile
  models deferred self-credit after the sender invocation.
  Unsupported outcomes and mismatches fail explicitly.
- Service upgrades reserve abstract hashes 9001 and 9002 for executable service
  blobs of exactly 262144 bytes. The codex changes and pads only opaque JAM
  container metadata; both blobs execute the current service implementation.
  Container headers use JAM codec, while upgrade messages use SCALE. The initial
  abstract hash zero retains its historical placeholder length of zero.
  Replay loads the installed hash for every invocation, including after an
  upgrade; missing or invalid installed code fails instead of falling back to
  the original binary. This tests upgrade mechanics, not migrations between
  implementations with different storage layouts.
  Upgrade traces must carry `replayMinAccGas` in every frame. The pinned model
  stores the code hash, code length, and minimum memo gas, but not minimum
  accumulate gas. The shared `messageEffects` observer records that argument
  from the last successful model `applyUpwardMessage` and carries it through
  rejected/no-upgrade frames. Replay compares all four fields. The host exposes
  final account fields, not a log of intermediate upgrade calls.
  A wrong upgrade length keeps the same code hash and reaches the availability
  check. Conflicting lengths in actual solicit/provision requests remain outside
  the codex domain. Generated upgrades target these executable blobs; the
  service's lack of validation for arbitrary replacement blobs remains the
  consensus-critical FIXME in `accumulate/upward.rs`.
- Accumulate-log decoding supports `ForgetAgainAt`, `StateBalanceUpdateRejected`,
  `TooMuchStateHeld`, `InvalidCodeHashAcc`, `CodeUpgradeNotAvailable`,
  `CodeUpgradeNotAnnounced`, `CanNotRemoveCode`, `CoreNotAssignable`,
  `DesignateRejected`, `StagedValidatorKeysOverflow`, `TransferFailed` (all six reasons),
  `ServiceUpgradePreimageMissing`, and
  `InsufficientStateBalance` from `FromSolicit` or `FromSetKV`; other events fail explicitly.
- `Solicit` and `Forget` support explicit parachain targets, including delegated
  calls, and historical fixtures without `Target`. Service targets replay the
  host's `UnknownService` and `NotSupervised` refusals. Seeded foreign accounts
  remain empty; created accounts retain their initial code request. Both account
  footprints and model requests are checked after every invocation.
  The compatibility model explicitly returns `NotSupervised` for self targets:
  the pinned model searches only `foreignServices` and returns `UnknownService`,
  while the host knows this service exists. Other targets use the pinned model.
  Successful foreign solicit/forget remains outside the host's capabilities
  ([DIVERGENCE.md M-11](../../../../../DIVERGENCE.md#m-11-the-model-decides-the-65-supervised-service-outcomes-rust-can-only-refuse)).
- Abstract hashes require a consistent preimage length within each domain.
  `solicitedSet` is model ghost state, not a returned JAM output.
- KV keys and values are literal byte lists. Failure-log key hashes use a separate
  codex for Quint's base-257 `listHash`; ambiguous hashes (such as empty and
  leading-zero keys) and hashes exceeding i128 fail explicitly. The generator
  uses a small collision-free key pool.

Gas and checkpoint fixtures live in `gas/`, generated from `gas.qnt`. They cover
report-budget boundaries (including deferred gas and refine errors), continuing
after a gas rejection, and rollback of partially applied reports after real VM
out-of-gas. See [FUZZING.md](FUZZING.md) for the replay-only gas model and limits.

`storage.qnt` provides host backstop fixtures: head and KV rejection, incoming
bucket and endpoint failures, cleanup of earlier buckets, credit and write
boundaries, and recovery on later reports/invocations. Metadata fixtures add
registration, compact-encoded balance growth, forced head/code writes, validator
staging boundaries, retained reference charges, abort refunds, and retries. `storage_inputs.qnt`
is a replay-only extension for the host deposit check. Failed head and queue
writes emit no parachain log; KV failures retain the spec's `SetKV` reason. `storage_fuzz.qnt` samples these inputs through
the regular streaming replay harness; see [FUZZING.md](FUZZING.md).

`host_mixed.qnt` covers KV backstop failures in the general campaign alongside
gas gates, checkpoint recovery, due assignments, upgrades, preimages, and
cleanup. `host_invocation.qnt` uses the pin for supported operations and extends
KV writes/log persistence with independently sized host deposits from
`host_sizes.qnt`. Default `fuzz.qnt` samples these invocations on the same state
as its ordinary actions; no extra profile selection is required.

`service_management.qnt` covers `RemoveServiceStorage` and
`SetServiceSupervisor`. The self-target compatibility rule also applies here:
the host knows self exists, while the pinned foreign-service map excludes it.
For supervisor handoff, a missing new supervisor precedes `NotSupervised`.
Every replay frame requires foreign services to remain self-supervised in the
compatibility model; the host has no mutable supervisor link. Successful storage
removal and supervisor handoff remain outside this host's replay domain.

Panic fixtures set `replayPanic` together with `replayInterrupt`. The replay
adapter replaces that report's output with an empty SCALE digest and requires
an actual PVM `Trap`. The fault occurs before the report's gas gate and state
writes. With `replayPanic` absent or false, interrupted traces retain the existing
out-of-gas semantics. Unexpected VM failures always fail replay.

Instruction gas tests reuse the `panic_recovery` fixtures' independently generated
prefix states, replacing panic injection with gas exhaustion. No expected state
is derived from the PVM probes. The default Rust fuzz worker also decorates mixed
traces with deterministic gas samples; raw Node/Quint output remains unchanged.
See [invocation gas cutoffs](FUZZING.md#invocation-gas-cutoffs) for metadata,
calibration, and limits.

Final-checkpoint exhaustion fixtures live in `final_checkpoint/`. They preserve
all reports' state and host effects while expecting `NotEnoughGas` with no head
commitment. Tests sample the final tail, bracket head reads, and resume after
repeated failures. Storage is persisted before the checkpoint in this runtime;
there are no final-tail writes. See the fuzzing guide for calibration details.

The `outgoing_boundaries` fixtures and `outgoing_boundary` generated profile cover
threshold-adjacent spends using an explicit host-budget compatibility oracle;
see the [fuzzing guide](FUZZING.md#outgoing-spendable-balance-boundaries).
