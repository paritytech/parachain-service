# Parachain Service on JAM: Graypaper Compatibility Issues

## Audit scope and verdict

This file records the full findings from comparing
`[parachain-service-on-jam.md](./parachain-service-on-jam.md)` with the adjacent JAM Graypaper,
version `[0.8.0](../../../graypaper/VERSION)`, and from exercising the accompanying
`[quint](./quint)` model.

The design is not yet a correct or deployable Parachain Service specification. It is a useful
architecture sketch, but several interfaces contradict JAM and major state, funding, messaging,
failure-handling, and migration mechanisms remain undefined.

## Blocking protocol and ABI issues



### 1. Refine has no authenticated parachain-state input

JAM Refine has no general chain-state access. It receives package data, imports/extrinsics, context,
authorizer trace, and historical preimages. The design nevertheless expects the PVF to read downward
transfers, logs, KV state, preimage availability, and other Parachain Service state "through
validation inputs," without defining those inputs or a state proof.

As written:

- Asset Hub cannot safely consume incoming transfers.
- Parachains cannot read data previously written through `kv_set`.
- Parachains cannot reliably read their service logs.
- Runtime and service-upgrade logic cannot observe authoritative service state.
- Slot and parent-state checks cannot bind execution to authenticated state.

A concrete validation-input format and proof scheme anchored to the context's
`lookup_anchor_post_state_root` is required.

References: design lines 556-559, 797-800, and 875-879; Graypaper
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 59-120 and
`[accounts.tex](../../../graypaper/text/accounts.tex)` lines 69-89.

### 2. The child-PVF ABI is not implementable as specified

`fn jam_validate_block() -> ()` does not define:

- the PVM program/preimage format;
- entry PC and initial memory;
- validation-input and result encodings;
- child host-call identifiers and memory conventions;
- child gas allocation and accounting;
- abnormal-exit and host-call error mapping; or
- compatibility/version negotiation.

JAM's `machine`, `peek`, `poke`, `pages`, and `invoke` APIs operate at a much lower level. Existing
parachains are WASM blobs called as `validate_block(ValidationParams) -> ValidationResult`; they
cannot use the proposed entry point without a new runtime toolchain and migration path.

References: design lines 548-574; Graypaper
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 69-120 and 590-710;
current SDK `[cumulus/test/client/src/lib.rs](../../cumulus/test/client/src/lib.rs)` lines 197-225.

### 3. The parent-head check authenticates only a declaration

The wrapper accepts whatever `set_parent_head_hash` the child supplies, while the actual parent head
and state used during validation are not defined or proven. Accumulate comparing that declaration
with the stored head hash is insufficient unless the validation ABI requires the corresponding
parent data and proof and binds all validation inputs to it.

References: design lines 602-603 and 686-689.

### 4. The JAM `assign` ABI is modeled incorrectly

Graypaper `assign` always takes:

- a valid core index;
- exactly 80 authorizer hashes; and
- a new assigner service ID.

The design instead permits queues of length `0..80`, separates `set_authorizer_queue` from
`set_assigner`, offers `None` to reset an assigner, and claims that it can retain the current
assigner. Those operations do not exist.

Once the current queue is drained from service storage, the service also lacks the 80 hashes
required to change only the assigner. Resetting privileges requires the manager's `bless`, not
`assign(None)`.

References: design lines 610-611, 1214-1229, and 1337-1339; Graypaper
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 762-778.

### 5. The single-slot on-demand model cannot work

An authorizer queue is an 80-entry cyclic schedule. A changed queue contributes only the entry at
`timeslot mod 80` to the pool after accumulation. Guarantees in that block are checked against the
prior pool. Once admitted, an unused authorizer may remain in the pool for up to eight blocks.

Consequently, `set_authorizer_queue(..., near_term_slot)` cannot install an authorizer for exactly
one slot. A real design must account for modulo-80 placement, report and availability latency, pool
admission, use/removal, and a later queue replacement.

References: design lines 1346-1366; Graypaper
`[authorization.tex](../../../graypaper/text/authorization.tex)` lines 13-30 and
`[reporting_assurance.tex](../../../graypaper/text/reporting_assurance.tex)` lines 323-334.

### 6. Required JAM privileges and bootstrap state are unspecified

For this architecture to operate, the Parachain Service must be:

- the assigner for every core it manages;
- the delegator for validator-set updates;
- an always-accumulate service with sufficient gas; and
- coordinated with the manager for privilege resets and any gratis balance.

Without explicit genesis/bootstrap transitions, `assign` and `designate` return errors, and
scheduled housekeeping does not run.

Reference: Graypaper `[accounts.tex](../../../graypaper/text/accounts.tex)` lines 163-180.

### 7. Per-parachain quotas are disconnected from the real JAM service balance

`total_state_balance` is only an internal storage number. `parachain_set_state_balance` transfers no
JAM balance, there is no invariant relating the sum of quotas to `service_account.balance`, and no
real funding or refund flow is defined.

Coretime can therefore allocate arbitrary headroom while JAM `write` or `solicit` returns `FULL`.
The design's claim that writes never fail on balance grounds is false as written. Storage collateral
and Asset Hub transfer liquidity also share the same JAM service balance without any partition or
reservation rule.

References: design lines 945-973 and 1110-1121; Graypaper
`[accounts.tex](../../../graypaper/text/accounts.tex)` lines 11-29 and
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 472-495 and 946-968.

### 8. State-deposit accounting is wrong

The accounting omits mandatory JAM balance components:

- JAM charges both octets and items.
- A preimage request counts as two items, and its registry storage entry is another item. Under the
design's sole-user rule, the incremental charge is `187 + len`, not `157 + len`.
- Every KV and other storage entry needs the additional item deposit.
- The two generic baseline storage entries need another 20 balance units.
- The service's one-time base deposit and `gratis` must be included in aggregate backing.
- The `ParaInfo` byte calculation omits the `Option<ValidationCode>` discriminant and
`is_deregistering`, undercounting it by two bytes.
- The design uses `Compact<u128>`, while JAM balances are `u64`. Private `u128` bookkeeping would
need explicit bounds and conversion rules.

References: design lines 935-943, 1011-1058, and 1086-1108; Graypaper
`[accounts.tex](../../../graypaper/text/accounts.tex)` lines 135-158,
`[definitions.tex](../../../graypaper/text/definitions.tex)` lines 258-260, and
`[overview.tex](../../../graypaper/text/overview.tex)` lines 105-108.

### 9. Parachain state commits before fallible JAM effects

The new head is installed before transfers, authorizer changes, validator designation, solicits, KV
writes, and service upgrades are replayed. These host calls can fail while the parachain block
remains enacted.

Unless every parachain runtime treats these requests as asynchronous and reconciles later receipts,
this permits failures such as:

- Asset Hub burning assets while the JAM transfer fails;
- Coretime recording a sale or assignment that JAM did not install;
- a runtime assuming validator rotation before `designate` succeeded; and
- a runtime assuming storage or preimage changes that were rejected by JAM.

The present lossy log is not a transaction or receipt protocol.

References: design lines 699-709 and 609-619; Graypaper
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 762-804 and 847-998.

### 10. The transfer API is incomplete and the queue protocol is unsafe

- JAM `transfer` requires destination memo gas; `transfer_out` has no such argument or policy.
- Bouncing a full-queue transfer is not guaranteed. The sender's minimum memo gas may exceed the gas
supplied to the Parachain Service invocation.
- Zero-value or tiny transfers can fill all 1,000 queue slots.
- `consume_transfers_up_to(index)` uses a position in a mutable vector. Earlier pruning can make a
later candidate consume newly arrived entries. A monotonic transfer ID and base cursor are needed.
- Inclusive/exclusive and out-of-range index behavior is undefined.
- `[0xFF; 128]` permanently reserves a valid memo without an application-level collision policy.
- Beneficiary, mint/burn, acknowledgement, retry, and failed-withdrawal reconciliation semantics are
missing.

References: design lines 607-613 and 657-666; Graypaper
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 864-893.

### 11. Checkpointing does not prevent loss of unprocessed results

Graypaper invokes a service once with all of its transfer and work operands and the aggregate gas
allocation. On panic or OOG, a checkpoint preserves earlier state, but execution does not resume and
later operands are not retried.

The implementation therefore needs strict per-operand metering and a proven worst-case gas bound.
"Checkpoint after each report" only protects earlier reports. The 10-million-gas per-report ceiling
and always-accumulate gas have not been reconciled with up to 1,024 side effects per result.

References: design lines 730-735; Graypaper
`[accumulation.tex](../../../graypaper/text/accumulation.tex)` lines 289-343,
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 219-247, and
`[definitions.tex](../../../graypaper/text/definitions.tex)` lines 265-268.

### 12. All Graypaper `WorkError`s are silently discarded

These errors include OOG, panic, bad exports, oversized output, unavailable service code, and
oversized service code. They are not all merely bugs in the service's Refine wrapper.

Losing them without a ParaId, trace, state transition, or log makes operational failures invisible
and consumes work without advancing the parachain. The authorizer trace or another protocol-level
mapping must identify the para even when no custom digest was produced.

References: design lines 515-519 and 670-673; Graypaper
`[reporting_assurance.tex](../../../graypaper/text/reporting_assurance.tex)` lines 114-122.

### 13. Service self-upgrade can permanently brick the service

- A JAM service code preimage must encode `(metadata, code)`, not merely an assumed SCALE code blob.
- `upgrade` validates neither availability nor program correctness.
- The design checks only that a registry referencer exists, not that a valid service program is
currently provided.
- There is no fallback code hash or external recovery path if the new code cannot execute.
- The current service code is not separately pinned as a service-owned reference.
- Already-guaranteed results produced by the old Refine code may later be decoded by the new
Accumulate code. There is no result/storage version or pipeline-drain protocol.

References: design lines 863-894; Graypaper `[accounts.tex](../../../graypaper/text/accounts.tex)`
lines 38-52, `[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 847-861, and
`[reporting_assurance.tex](../../../graypaper/text/reporting_assurance.tex)` lines 428-431.

### 14. Messaging is not actually specified

The document explicitly leaves messaging host functions TBD, and the work digest has no XCMP header
or message variant. Missing pieces include:

- channel creation, acceptance, closure, and ownership;
- sequence numbers and watermarks;
- routing and destination discovery;
- congestion, fees, and admission control;
- acknowledgements and replay protection;
- data retention, receiver recovery, and expiry; and
- migration of current HRMP/UMP state and semantics.

D3L is not a transparent off-chain message bus:

- each export is exactly 4,104 bytes after padding or truncation;
- larger messages require deterministic chunking;
- consumers import by segment root and index with proofs, not by message hash alone;
- export count must be declared in advance; and
- guaranteed retention is only a minimum of 28 days.

Therefore current HRMP/UMP migration, "full XCMP," and the XCM-dependent collator-set rotation flow
cannot work yet.

References: design lines 1370-1412; Graypaper
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 572-587 and
`[work_packages_and_reports.tex](../../../graypaper/text/work_packages_and_reports.tex)` lines 45-57
and 154-168.

## Other real-world correctness and liveness problems



### 15. Forced management operations race with normal candidates

A target candidate can be accepted and apply effects, followed later in the same accumulation
invocation by Coretime forcibly overwriting its head or code. Reversing report order rejects the
candidate instead. Cleanup and quota changes have similar order dependence.

Forced recovery needs a generation/epoch nonce or explicit fencing state that prevents normal
candidate effects from being enacted concurrently with management changes.

References: design lines 668-735 and 1147-1178.

### 16. Upgrade timeout cleanup is lazy

A pending validation-code upgrade is reaped only while processing another candidate whose parent
check succeeds. A dead or idle parachain therefore holds the preimage forever, contradicting the
stated timeout protection.

A deadline index processed by always-accumulate is needed. The policy must also clarify whether a
report refined or guaranteed before the deadline should be rejected merely because availability or
dependency processing delayed accumulation until after it.

References: design lines 690-698 and 809-829.

### 17. A dead parachain cannot be forcibly deregistered

Cleanup refuses to proceed until the parachain itself has removed every KV entry and arbitrary
solicited preimage. A bricked, malicious, or abandoned runtime cannot do so, leaving state and
deposits locked indefinitely.

The storage layout has no per-para enumerable key/preimage index and no bounded administrative
sweeper.

References: design lines 1180-1207.

### 18. Registration is not atomic and has no two-phase acknowledgement

The three management messages can partially succeed: quota creation can create an incomplete
`ParaInfo`, the head can be installed, and then code solicitation/setup can fail. Coretime has
already committed its own block and receives only a later, evictable log.

Registration, funding, core assignment, readiness, and refunding need an explicit state machine and
durable operation identifiers.

References: design lines 1123-1145.

### 19. The preimage lifecycle has unresolved cases

- The current Parachain Service code lacks an independent service-owned pin.
- A claimed hash with the wrong length creates a request that can never be supplied.
- "Same hash, different lengths are different codes" is misleading. Only the request records differ;
a hash identifies one actual blob and length.
- Rescue and forget behavior requires explicit `query` handling, but the prose claims no bookkeeping
and does not give a complete return-code algorithm.
- Evicted `ForgetAgainAt` logs can strand cleanup callers.
- Forced replacements can leave several charged tombstones that only the affected parachain can
finish forgetting.
- There is no maximum inner PVF length or validation policy before forced activation.

References: design lines 737-829 and 975-1015; Graypaper
`[pvm_invocations.tex](../../../graypaper/text/pvm_invocations.tex)` lines 921-998.

### 20. The authorizer does not enforce the intended work target

The AURA example verifies the collator but does not require every item to target the Parachain
Service, validate expected Refine/Accumulate gas policy, or constrain imports and exports.

If para-specific coretime is intended to be usable only for that para, a buyer can instead authorize
arbitrary JAM work. If coretime is deliberately transferable to arbitrary services, that policy
needs to be stated explicitly.

References: design lines 1287-1306.

### 21. The AURA example is not a complete slot protocol

The design deliberately allows selecting any recent anchor and admits duplicate or consecutive
claims. Its proposed fix depends on the missing authenticated validation inputs.

It also lacks:

- rejection of zero `slot_duration` and `collator_set_size`;
- canonical Merkle-tree and non-power-of-two rules;
- a domain-separated canonical encoding for the token-free package hash;
- authorizer-code hosting, solicitation, and retention rules; and
- a complete evidence, identity, bonding, dispute, and slashing protocol.

References: design lines 1231-1324.

### 22. The 4 KiB head cap is incompatible with all currently valid parachains

The SDK's hard supported limit is 1 MiB, and configurations can use larger values such as 32 KiB. A
migration must either prove that every target chain is below 4 KiB or use a larger or committed head
representation.

References: design lines 309-311; SDK
`[polkadot/primitives/src/v9/mod.rs](../../polkadot/primitives/src/v9/mod.rs)` lines 450-456 and
`[cumulus/pallets/parachain-system/src/lib.rs](../../cumulus/pallets/parachain-system/src/lib.rs)`
lines 1643-1650.

### 23. Logs cannot serve as reliable receipts or slashing evidence

- Logs are capped, pruned, and evicted.
- Authorizer traces are truncated to 256 bytes.
- Transfer failures preserve only a memo hash.
- Parent-head rejection and Graypaper `WorkError` produce no log.
- Management failures from multiple target paras are grouped under the originating Coretime work
result.
- Several events omit the target ParaId, operation ID, and exact JAM error code.

Financial, control-plane, cleanup, and slashing correctness cannot depend on this log.

References: design lines 166-215, 263-301, and 711-728.

### 24. Several large or shared values have no practical gas bound

`incoming_transfers`, `staged_validator_keys`, logs, and especially a shared `preimage_registry`
referencer set are monolithic encoded values. Appending or removing an entry can require reading,
decoding, and rewriting the whole value.

The referencer set is described as bounded by a protocol-wide parachain maximum, but no usable
implementation bound is specified. These structures need benchmarks against the 10-million-gas
report ceiling and likely need paging or per-membership storage.

References: design lines 154-195 and 1060-1084.

### 25. The prose contains implementation-significant contradictions

- Refine failures say that no log pruning occurs, while the general rule says every candidate prunes
before appending.
- `used_state_balance` is documented as tracking only PVF preimages but later includes baseline
state, logs, KV data, and global reserves.
- `ParaInfo` has concrete non-optional fields while registration says those fields are initially
"uninitialized."
- `OnTransfer` is stale terminology for Graypaper 0.8.0, where deferred transfers are operands to
the single Accumulate invocation.

References: design lines 330-348, 679-728, and 952-957; Graypaper
`[accumulation.tex](../../../graypaper/text/accumulation.tex)` lines 289-341.

## Quint-model defects



### 26. Solicited-but-unprovided validation code is treated as available

`codeAvailable` checks only that a registry referencer exists. It ignores `Unprovided`,
`Unrequested`, and the lookup-anchor time. Initial active codes can therefore execute in the model
before anyone provides their preimages.

References: `[quint/state.qnt](./quint/state.qnt)` lines 43-63 and
`[quint/refine.qnt](./quint/refine.qnt)` lines 140-158.

### 27. Service upgrade repeats the same availability bug

The model upgrades whenever a registry referencer exists, even when the code preimage is
`Unprovided` or unavailable. It can therefore model a successful upgrade that would leave the real
JAM service without executable code.

Reference: `[quint/accumulate.qnt](./quint/accumulate.qnt)` lines 296-307.

### 28. Important effects are stubbed or model the wrong Graypaper API

- `TransferOut` is a no-op.
- Assignment is split into impossible queue-only and assigner-only operations.
- The queue invariant permits fewer than 80 entries.
- Authorization pools and their block transition are absent.
- Actual JAM privileges and host-call return codes are absent.
- Actual service balance, `FULL`, `LOW`, `CASH`, and related failures are absent.
- Gas, OOG, panic, and checkpoint collapse are absent.
- D3L and messaging are absent.

References: `[quint/accumulate.qnt](./quint/accumulate.qnt)` lines 233-285 and
`[quint/invariants.qnt](./quint/invariants.qnt)` lines 342-369.

### 29. The 48 KiB result-budget model is incorrect

It assumes a 256-byte maximum authorizer trace, confusing stored-log truncation with the full report
trace. It also omits result fields and encoding overhead, including code length, lookup-anchor
timeslot, and enum/container overhead.

Graypaper limits the exact successful output blob plus the full authorizer trace.

References: `[quint/refine.qnt](./quint/refine.qnt)` lines 98-106; Graypaper
`[reporting_assurance.tex](../../../graypaper/text/reporting_assurance.tex)` lines 124-134.

### 30. The model omits the boundaries where most protocol risk lies

The model explicitly abstracts or omits cryptography, real PVF execution, D3L, AURA, anchor-state
proofs, lookup-anchor state access, and messaging. It also omits actual JAM item deposits from its
balance model.

References: `[quint/README.md](./quint/README.md)` lines 24-46 and
`[quint/state_balance.qnt](./quint/state_balance.qnt)` lines 38-48.

### 31. The advertised randomized invariant does not hold

The following command reproducibly exits with an invariant violation:

```sh
cd designs/parachain-service-on-jam/quint
quint run main.qnt \
  --invariant=parent_head_continuity \
  --max-steps=30 \
  --max-samples=10000 \
  --seed=0x513689 \
  --backend=rust
```

The reproduced counterexample is primarily an invariant bug rather than proof of a service-state
bug. The invariant replay accepts every result with a matching parent but ignores the real
accumulator's validation-code rejection. A preceding result can force a code change, making the
following old-code result invalid even though its parent matches.

Reference: `[quint/invariants.qnt](./quint/invariants.qnt)` lines 227-263.

### 32. Passing unit tests do not establish Graypaper compatibility

The model typechecks and all 39 scripted tests pass:

```sh
cd designs/parachain-service-on-jam/quint
quint typecheck main.qnt
quint test tests.qnt
```

Those tests exercise the abstract model and therefore encode several of the incorrect or incomplete
assumptions listed above.

## Documentation drift



### 33. The table of contents and model references are stale

The table of contents promises "§9 Missing JAM / Gray Paper Features" and "§10 References," but §9
is References and the missing-features section does not exist. The Quint README still refers to the
absent §9, while the design's TODO section is empty.

References: design lines 27-32 and 1416-1430; `[quint/README.md](./quint/README.md)` lines 44-46.

## Required work before implementation

At minimum, implementation should not begin until the design has:

1. A versioned, authenticated validation-input and child-PVM ABI.
2. Correct `assign`, privilege, authorizer-pool, and queue-timing semantics.
3. Real service-balance escrow, aggregate collateral accounting, and refund flows.
4. Durable, operation-ID-based asynchronous receipts for every fallible side effect.
5. Per-operand gas isolation and a benchmarked worst-case Accumulate path.
6. Safe service-code and validation-code upgrade/recovery protocols.
7. A force-cleanup path for dead parachains.
8. A complete XCMP/D3L channel, segment, acknowledgement, and retention protocol.
9. A repaired formal model that includes historical availability, actual Graypaper host-call
  semantics, gas/failure behavior, privileges, and real balance constraints.
