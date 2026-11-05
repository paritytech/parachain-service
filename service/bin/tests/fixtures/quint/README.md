# Quint replay fixtures

Combined fixtures execute Rust Refine and pass its actual outputs to Rust
Accumulate in the PVM. Historical accumulate fixtures supply recorded work
results directly. Both compare storage and returned head commitments.
For streaming campaigns and saved-failure replay, see [FUZZING.md](FUZZING.md).

## Regenerate and replay

From the repository root, with Python 3 and Quint **0.32.0** on PATH:

```sh
python3 scripts/generate-quint-replays.py
cargo test -p parachain-service-bin --test quint_replay
```

The generator uses the pinned model, TypeScript backend, and seed 1. It regenerates
`authorization`, `combined`, `refine_fuzz`, `refine_errors`, `blocks`, `upgrades`, `log_pruning`, `kv`, `balances`, `lifecycle`,
`assignments`, `validator_keys`, `outgoing`, `service_upgrades`, `mixed`, `gas`,
`storage`, `storage_recovery`, `code_recovery`, `host_mixed`, `services`, `service_preimages`, `service_management`, and `panic_recovery` scenarios,
including the root minimal and stale-parent fixtures. Other historical fixtures
are retained.
JSON is compact and timestamp-free; use `just quint-fmt` to expand it for review
and `just quint-compact` before committing.

## Coverage

| Scenarios | Checks |
| --- | --- |
| Combined refine and accumulate | Original scripts, actual digests, ordered batches, errors, and checkpoint recovery |
| Refine errors, WorkErr, empty blocks | Error logs, unlogged failures of other code, and unchanged state for skipped work |
| Pre-checkpoint gas recovery | Independent rollback oracle, retained incoming funds, reverted due assignments and queue writes, repeated exhaustion, and successful retries |
| Instruction gas recovery | Sampled invocation limits and cutoffs before/after writes, creation, and transfers; fixed Quint prefix, checkpoint effects, and subsequent invocations |
| Due-flush recovery | Injected gas exhaustion before/after assignment, read, and write calls in funded initial/final flushes; rollback of three cores, rotating queues, retained reports/credits, and retries |
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
  Hazmat observation additionally compares each successful designation in
  model order, including repeated identical sets and overwritten calls. The `designation` profile extends
  the model with explicit `replayCanDesignate` inputs and covers privilege rejection
  and restoration. The general fuzz input pool includes privileged key updates.
  Unexpected provide or eject effects fail; creation and transfers use the explicit oracles below.
- Assignment messages and initial pending queues are supported. Authorizer integers
  map to a u32 little-endian prefix padded to 32 bytes. Assignment service IDs swap
  Quint 1 with mock 0; all other IDs remain literal (incoming-transfer IDs retain
  their existing literal mapping). Replay starts with this service owning every
  core and carries actual JAM privileges between blocks, including handoffs.
  Quint's ordered `lastStepAssigns` is folded into expected final queues and
  privileges, checking all privilege fields and rejecting extra assigned cores.
  Hazmat host observation additionally records every successful assignment,
  including arguments and order across cores and repeated calls to one core.
  Completed checkpoints mark the retained prefix on rollback. Final queues and
  privileges are still compared independently. The model's `jamCoreAssigners`
  ghost state is checked through those privileges.
- Due assignments are flushed in due-slot/core order within the available
  allowance, retried after work reports using leftover gas, and otherwise
  carried into later blocks (§5.1). Assignment regression tests cover partial
  flushing, carry-over, rotation, and rejection log order. Interrupted replay
  invocations select a zero assignment budget when no allowance remains;
  rollback before the first checkpoint retains no assignment effects.

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
  rejected/no-upgrade frames. Replay compares all four fields. Hazmat observation
  also compares every successful self-upgrade call, including intermediate code
  hashes and gas settings overwritten within the invocation.
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

## Refine execution

The default `fuzz.qnt` campaign retains refine inputs and runs Rust Refine before
Rust Accumulate. `combined.qnt` and `combined/` contain end-to-end fixtures.
`refine_fuzz.qnt` and `refine_fuzz/` retain focused standalone boundary regressions;
`refine_bytes.qnt` separately samples empty/truncated/trailing candidate payloads and
empty/bad-magic/truncated PVF binaries, plus structurally valid containers with
invalid memory maps or missing entry points. The adapter checks that these
containers reach their intended parse failures. Their whole-work failure expectations
come from an explicit local Quint boundary extension (§4.1/4.2), because the
upstream typed model cannot express malformed bytes. Missing-preimage fixtures
check lookup-before-PVF-parsing precedence; a later valid invocation checks recovery.
Mutation tests detect bypassed byte faults, and generated coverage requires every
fault plus the existing Refine outcomes. `combined/malformed_works.itf.json`
checks failed-only batches, mixed valid/failed work, and later valid invocations.
`refine_bytes_pipeline.qnt` generates 10 seeds of 20 blocks, with malformed work
before and after valid work in each block; coverage requires all eight faults.
Both stages compare against independent Quint expectations, including retained
Accumulate state. This does not cover every structural PVF parser failure or
arbitrary malformed byte distributions.
`refine_gas.qnt` samples zero entry gas and a scripted infinite child loop with a
finite 1,000,000 gas budget. Its local oracle predicts whole-work failure without
calibrating against Rust outputs. A funded control executes the same declarations
without the loop at that budget; missing-code cases check lookup precedence, and
fixtures execute a funded call after exhaustion. Mutations remove the gas fault
and must disagree with the oracle. Ordinary traces still reject unexpected gas
errors. `combined/gas_cutoffs_works.itf.json` now carries entry, child, and
finite-script gas failures through Accumulate, with funded candidates and later
retries. Finite cutoffs sample five budgets from zero to one below a successful
control's measured cost. Quint independently predicts failed work and retained
state; gas calibration supplies neither. `QUINT_FUZZ_PROFILE=refine_gas` extends
this to generated mixed batches, requiring all three modes and five samples.
Arbitrary PVFs and exhaustive instruction cutoffs remain outside this domain.
`refine_errors/` supplies errors directly to accumulate. Regenerate fixtures with
`scripts/generate-quint-replays.py`. All use the same parser and saved-failure
replay entry point; see [FUZZING.md](FUZZING.md).

## Storage recovery combinations

The `storage_recovery` profile combines the independent storage-deposit oracle
with sampled PVM exhaustion before the first checkpoint, inside either report,
and after the final checkpoint. It checks head/KV and queue rejection,
registration/forced-head metadata, validator staging, retained incoming credits,
checkpointed state/free balance, and successful later retries. Expected state
is calculated by applying the Quint storage oracle only through the selected
report prefix; gas probes select cutoffs without supplying expected state.

Focused fixtures sample five positions per interval. The opt-in generated test
requires every recovery phase, metadata and head failures in interrupted
invocations, zero-headroom checkpointed validator finalization, and subsequent
successful invocations. Balance fixtures cross all reachable compact-u64 size
boundaries (2^30, 2^32, 2^40, 2^48, 2^56), checking rejected growth, rollback,
one-byte funding, shrink refunds, and retries. Validator fixtures retain or roll
back both the staged-buffer deposit release and the designation. Assignment
fixtures cover scheduled payload/index writes, initial index creation, failed
replacement growth, shrink refunds, index-failure deposit refunds, and funded retries.
Generated coverage requires checkpointed rejection at both assignment write sites.
Inline fixtures cover post-call payload/index rejection, rollback of successful
calls, retained calls despite rejected cache growth, funded replacement, zero-balance
tiling queues, and later due flushing of preserved or rotated queues. Generated
coverage requires both inline write refusals with retained calls and a due flush.
Sampled gas invocations exclude already-due entries: calibration removes the
initial flush allowance. Due-flush checks use fully funded invocations.
The separate `due_flush` fixtures below inject faults into active flushes.
Other upward-message backstop sites remain follow-ups in
[the coverage TODO](../../../../../QUINT_REPLAY_COVERAGE_TODO.md).

The `code_recovery` profile applies the independent code-storage oracle to a
single-report invocation with pre-checkpoint, in-report, and final-checkpoint
exhaustion. It checks forced-code registry/acquisition and metadata rejection,
announcement writes, retained references and free balance, and successful retries.
Natural solicitation traps remain distinct from sampled gas exhaustion.

The `host_mixed/{cleanup_partial,cleanup_full,remove_refund,forget_refund,code_apply}_recovery`
fixtures cover instruction exhaustion during bucket deletion and endpoint
persistence, transfer-queue charge refunds, KV deletion/refund, unprovided
preimage forgetting/registry deletion/refund, and announced-code activation.
The cleanup setup exceeds reserved queue capacity, so both partial and full
cleanup actually refund a charge. Later invocations retry the operation, reuse
released storage, restart the emptied queue, or execute with the activated code.

`write_recovery.rs` checks both sides of every write host call in the interrupted
report, using zero-based `replayGasBoundary` occurrences (for example,
`after:write:1`). It also samples five instruction cutoffs inside the report and
after its final checkpoint. Quint computes rollback and committed-prefix states
before calibration; probes use only host-call order and gas. Required minimum
write counts prevent vacuous coverage, and changing the selected checkpoint must
fail against the original oracle. These are focused fixtures, not exhaustive
instruction enumeration or generated combinations of all cleanup lifecycles.

The `due_flush` fixtures cover three pending cores with one-hash tiling queues
and three-hash rotating queues. `due_flush.rs` checks both sides of every assign,
read, and write in the selected initial or final phase. Initial faults preserve
incoming scheduler credits but roll back all guest changes. Final faults retain
the report checkpoint and roll back subsequent assignments and cache mutations.
A later invocation must flush all three cores with the original queue contents.

`replayDueFlushBoundary` selects `initial|final:before|after:assign|read|write:N`,
where N is a zero-based occurrence. A discarded calibration run locates calls
without reading recovered state. The hazmat-only, thread-local host hook drains
gas at that boundary and returns `NotEnoughGas`; the installed guest is unchanged.
Final-flush calibration suppresses the initial allowance while leaving enough
report gas for the end flush. These are **injected host-boundary faults**, not
evidence that ordinary flush costs exceed their reserved gas. Natural exhaustion
and arbitrary instruction cutoffs remain separate coverage gaps. Tests reject
unreachable boundaries, mismatched checkpoint phases, and retained assignments
that contradict Quint, and check that fault scopes are cleared after unwinding.

## Ordered designation and upgrade calls

`host_call_inputs.qnt` observes successful effects from model message transitions.
`replayHostCalls` preserves designation/upgrade order and duplicates across reports;
refused messages and rejected reports emit no entries. The mixed host-budget
oracle observes upgrades at its own message transitions. Designation privilege
fixtures retain only successful calls. No history is inferred from final keys,
code hashes, account fields, or Rust outputs.

The hazmat host records successful calls and completed checkpoints. Rust compares
the retained history after normal completion or checkpoint rollback, alongside
existing final-state comparisons. `host_calls/` covers interleaved calls,
identical repetitions, overwritten effects, traps, sampled gas exhaustion before,
inside and after report checkpoints, and subsequent successful invocations.
Mutation tests alter history without changing the expected final state.
New generated traces set `replayHostCallsRequired`; missing history then fails
rather than silently falling back to final-effect-only replay. Legacy saved
traces without that marker or history remain supported.

## Authorization pipeline

`authorization.qnt` executes the real ed25519 authorizer before Refine and
Accumulate. Its independent admission oracle covers a valid signature, corrupt
signature/key proof, changed signed context, wrong target service, mismatched
para count, zero collator count/slot duration, and empty/truncated/trailing
configuration and token encodings. Rejected packages produce no work report.
Accepted packages pass their actual author trace and actual Refine output to
Accumulate, including logged PVF errors, PVF traps, and later successful work.

Empty/truncated/trailing traces are injected **after** successful authorization
at the Refine boundary; they are not possible outputs of this authorizer. Refine
must trap, producing a WorkErr that leaves the parachain state unchanged. The
report still carries the original IsAuthorized trace.

The profile uses one fixed collator and a single-leaf Merkle tree. It signs the
final package, including the lookup anchor. The symbolic auth-trace value `-1`
uniquely maps to that collator's exact 32-byte public key; nonnegative values keep
the existing repeated-byte length mapping. This profile does not apply the
length-based storage-failure oracle to that symbolic value. It checks exact
stored trace bytes, not just their length. Mutations test admission, missing
packages, bypassed authorization, incorrect report traces, and bypassed malformed
trace validation. Multi-collator rotation/proof depth and authorization gas
exhaustion remain outside this replay domain (separate Rust tests cover collator
selection).

`authorization_fuzz.qnt` uses the same pipeline with randomly paired faults and
PVF outcomes. The opt-in generated test requires every fault, success/error/trap
outcomes, and both mixed and rejected-only invocations; it is also available as
`QUINT_FUZZ_PROFILE=authorization` for sustained campaigns.

The `host_mixed/kv_charge_*` fixtures isolate the two writes in `SetKV` at
SCALE's 2^30 used-balance boundary. An unprovided solicitation raises the charge
without allocating the requested preimage. Independent footprint arithmetic
predicts first-write rejection, second-write rejection with charge rollback,
and exact-fit success (52 units for insertion, 2 for a one-byte overwrite).
Removal and shrinking overwrites check refunds across that boundary. Recovery
samples five instruction cutoffs before or after the report checkpoint, retaining
only the Quint-selected prefix; a fully funded follow-up checks continued use.
Budget mutation tests require the rejected and committed states to disagree.
These fixed cases inject work results directly and do not cover every compact
width or every instruction cutoff.

`kv_charge_fuzz.qnt` adds a streaming `QUINT_FUZZ_PROFILE=kv_charge` campaign.
Its required regression runs 100 seeds through insertion/overwrite crossings at
2^30, 2^32, and 2^40, with value growth of 1, 2, or 64 bytes. The 2^40 setup
splits the reservation across 256 distinct, unprovided u32-sized requests.
The adapter maps reservations larger than 16 MiB to domain-separated hashes of
the abstract identity and length, without constructing synthetic bytes. Actual
provision/materialization above 16 MiB is explicitly rejected; smaller preimages
retain their content-based mapping.
Every trace checks zero, charge-only, and one-unit-short budgets, rollback of an
exact-fit write before its checkpoint, retained exact-fit success, zero-budget
shrink/removal refunds, and a successful retry. Rejection, retained-write, and
refund frames also exhaust gas after the report checkpoint. The regression
cycles through five gas samples, including both interval endpoints, with Quint
selecting the retained prefix independently of Rust gas calibration.
Coverage assertions require all eighteen width/mode/growth combinations,
expected rejection counts, and boundary-adjacent charges after recovery and
retry; Rust state and host free balance are compared against the Quint oracle.
These cases still inject work results and do not cover every instruction cutoff
or compact widths above 2^40. Use at least eleven steps to include the final refund.

The remaining upward-message write audit distinguishes growing writes from
shrinking ones: code application clears announcement metadata; KV removal,
reference release, and cleanup delete entries or refund balances. These paths
cannot require extra storage deposit, but still warrant combined instruction-gas
recovery coverage. Growing registration/head/balance/code-acquisition metadata,
validator staging, and assignment payload/index writes have dedicated backstop
fixtures; this does not imply exhaustive combinations across those domains.
