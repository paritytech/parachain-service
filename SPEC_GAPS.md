# Spec gaps — Parachain Service on JAM vs. Graypaper 0.8.0

## Audit scope

This file tracks unresolved specification issues in the current
[Parachain Service design](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/parachain-service-on-jam.md)
and its [Quint model](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint), checked against
the vendored JAM Graypaper, version [0.8.0](vendor/graypaper/VERSION). See
[VENDOR.md](./VENDOR.md) for the pinned revisions.

This is deliberately a list of current, actionable gaps. Resolved findings and general implementation
TODOs are not retained here. The Rust implementation is consulted where noted, but its vendored
[PolkaJAM types](vendor/polkajam/crates/jam-types/src/simple.rs) still declare Graypaper version 0.7.2;
those comparisons identify local design/implementation drift rather than establish complete
implementation compatibility with Graypaper 0.8.0.

The design has improved substantially since the previous audit. The remaining issues are concentrated
around the consensus ABI, JAM authorization timing and privileges, economic backing and transfer
reconciliation, upgrade and cleanup liveness, messaging, and fidelity of the formal model.

Gaps are ordered by severity, most impactful first — both across the tiers and within each tier.

## Critical — fund loss, silent divergence, or unrecoverable state

### 1. Refine lacks a specified authenticated service-state input

JAM Refine has no general access to service storage. The candidate payload now includes an opaque PoV
described as “block data + witness,” and the context exposes anchor state roots, but the design does not
specify which Parachain Service state is witnessed, the proof encoding and key set, snapshot semantics,
or how that proof is verified against the appropriate context root.

This state input is nevertheless relied upon later: parachain runtimes are expected to inspect
preimage availability, while Asset Hub is expected to read incoming transfers and `parachain_log`
entries through “validation inputs.” Without a canonical authenticated input, a PVF cannot safely act
on those values and validators cannot deterministically reproduce its view. The wire format, proof
verification, size/gas bounds, and child-PVF access ABI need to be specified.

References: design §3.2, §4.2–§4.3, §5.2, and §5.4; Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), “Refine Invocation,” and
[accounts.tex](vendor/graypaper/text/accounts.tex), “Historical Preimage Lookups.”

### 2. Transfer gas, admission, and financial reconciliation remain incomplete

The bucketed timeslot cursor fixes the old mutable-vector indexing problem. The remaining transfer
issues are:

- JAM `transfer` requires destination memo gas, but `transfer_out` carries no gas value or lookup policy.
- The state stores only bucket links, not a total queued-transfer count, so deciding whether an arrival
  is inside the reserved portion requires an unbounded scan or an unspecified counter.
- A same-timeslot bucket contains an unbounded vector. Appending rewrites the growing bucket, so transfer
  amount may cover the extra state deposit but does not bound execution gas. No fixed `min_memo_gas` can
  cover the worst case without a per-bucket bound or an O(1)-append layout.
- Once the reserved incoming portion is full, JAM has already credited an incoming transfer even when
  the service drops its queue record. No mint, refund, beneficiary, or trapped-funds recovery semantics
  are defined for that case.
- Zero or tiny transfers can consume the pre-provisioned entries and force later small transfers into
  the unrecorded path.
- Failed outbound transfers have only an evictable, memo-hash log; Asset Hub's burn/refund/retry protocol
  is not specified.

`min_memo_gas`, queue accounting, and the reserved queue size therefore still require a bounded layout
and worst-case benchmarks, as the design itself notes.

References: design §5.1 and §6.1; Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), `transfer`.

### 3. Checkpointing does not establish a worst-case Accumulate path

JAM invokes a service once with all transfer and work operands and one aggregate gas allocation. A
checkpoint preserves earlier effects after a later panic or out-of-gas exit, but execution does not
resume and later operands are not retried.

The design says to checkpoint after each report, but does not prove that one accepted digest fits its
gas allocation. A digest can contain up to 1,024 effects, and large values such as the staged validator
set, a 64 KiB log, and a shared preimage referencer set can require substantial decoding and rewriting.
Concrete size bounds, per-operand metering, and worst-case benchmarks are required.

References: design §4.3, §5.1, and §5.3; Graypaper
[accumulation.tex](vendor/graypaper/text/accumulation.tex), “The Accumulation Function,” and
[definitions.tex](vendor/graypaper/text/definitions.tex), `C_reportaccgas`.

### 4. Per-parachain quotas are not backed by the real service balance

`total_state_balance` and `used_state_balance` are private accounting values. The design says the
Coretime chain owns deposits and refunds, but `parachain_set_state_balance` transfers no JAM balance and
there is no invariant connecting:

- the sum of per-parachain reservations;
- Asset Hub's global reservation;
- the service base deposit and `gratis`; and
- the Parachain Service account's actual JAM balance.

Consequently, private headroom can exist while a real `write` or `solicit` returns `FULL`. The funding,
escrow, refund, and insolvency-recovery flows need to be defined. The wire types also need correction or
checked conversion: JAM `Balance` is `u64`, whereas the design's sizing and message comments assume
`Compact<u128>`.

References: design §6.1; Graypaper [accounts.tex](vendor/graypaper/text/accounts.tex), “Account Footprint
and Threshold Balance”; [jam-types `Balance`](vendor/polkajam/crates/jam-types/src/simple.rs).

### 5. Service self-upgrade lacks a safe compatibility and recovery protocol

The prose says Accumulate verifies that the new preimage is present, but does not define the exact
`lookup`, length check, or decoding step; the Quint model implements the check as only
a non-empty registry referencer set. JAM `upgrade` itself does not validate that the hash resolves to a
well-formed `(metadata, code)` service blob. The flow needs to require a currently available preimage and
decode it as the canonical JAM service blob before upgrading.

It also lacks a fallback/recovery authority if the new code cannot execute. Results refined and
guaranteed under the old service code may be accumulated after activation by the new code, but no digest
or storage version and no pipeline-drain/compatibility rule is specified.

References: design §5.4; Graypaper [accounts.tex](vendor/graypaper/text/accounts.tex), “Code and Gas,”
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), `upgrade`, and
[reporting_assurance.tex](vendor/graypaper/text/reporting_assurance.tex), “Contextual Validity of Reports.”

## High — security boundaries and deployment blockers

### 6. The implemented child-PVF ABI and the design do not yet agree

There is now a working inner-PVM path, so the old claim that the child PVF was not implementable was
stale. However, the consensus-facing ABI still needs one canonical, versioned definition.

The design specifies `jam_validate_block() -> ()` plus individual host functions. The implementation
currently exports `validate_block(ptr, len) -> (ptr, len)`, loads a PolkaVM `ProgramParts` memory image,
and assigns numeric child-host-call indices in code. The two descriptions also differ on how the parent
head and new head are returned.

The specification should pin:

- the accepted program-blob format and exported symbol;
- register, pointer, memory, and return-value conventions;
- every child host-call identifier and encoding;
- child-gas allocation and charging;
- exit/error mapping; and
- ABI version negotiation for runtime upgrades.

References: design §4.2–§4.3; [service/src/pvf.rs](service/src/pvf.rs) and
[runtimes/frameless/src/lib.rs](runtimes/frameless/src/lib.rs); Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), “General Functions.”

### 7. The AURA authorizer remains an example, not an executable protocol

The example does not fully constrain the work being authorized. In particular, it does not require work
items to target the Parachain Service or constrain expected service code, gas, imports, and exports, so a
collator can spend para-specific coretime on other JAM work if policy intended otherwise.

It also needs exact rejection rules for zero `slot_duration`/`collator_set_size`, canonical Merkle proof
and non-power-of-two tree semantics, and a canonical domain-separated token-free package encoding. The
current authorizer implementation still marks the round-robin selection and real proof/signature checks
as unfinished.

References: design §7.1; [authorizer/src/aura.rs](authorizer/src/aura.rs) and
[authorizer/src/is_authorized.rs](authorizer/src/is_authorized.rs).

### 8. Forced management updates are not fenced from in-flight candidates

Normal candidates and Coretime management messages are processed in operand order. An old candidate may
enact its head and side effects just before a forced head/code replacement; if it appears after the
replacement, the parent or code check rejects it instead. The latter is safe, while the former can leave
side effects from the state being recovered away from.

Forced recovery and deregistration need an epoch/generation fence or a documented core-drain procedure
covering already-guaranteed and not-yet-accumulated work.

References: design §5.1 and §6.2–§6.4.

### 9. `assign_core` still does not define a valid JAM queue or on-demand schedule

The updated design correctly combines queue and assigner changes into one operation. Two issues remain:

- JAM `assign` reads exactly 80 authorizer hashes, while the design and Quint model accept any non-empty
  list of at most 80. The service must reject invalid lengths or define exactly how a shorter policy-level
  list expands into an 80-entry `AuthQueue`.
- Applying a queue at `jam_slot` does not reserve that exact slot. At the end of a block, only the entry
  at `timeslot mod 80` is admitted to the eight-entry pool; guarantees in that block were checked against
  the previous pool. An unused authorization can remain in the pool, and the queue repeats every 80
  slots until replaced.

The on-demand flow therefore needs explicit queue construction, lead time, pool-admission/use timing,
and a later replacement policy.

References: design §4.3, §5.1, and §7.2; Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), `assign`, and
[authorization.tex](vendor/graypaper/text/authorization.tex), “Pool and Queue.”

### 10. Required JAM privileges and bootstrap state are not specified

The Parachain Service must be the assigner for every core it manages, the delegator if it calls
`designate`, and an always-accumulate service with enough gas for housekeeping. The manager must also
establish any gratis allowance and recover privileges when required.

The design describes those roles informally but does not define the genesis/bootstrap transition or
the authority hand-off between the manager, Coretime chain, Asset Hub, and Parachain Service. Without
that state, `assign`/`designate` fail and scheduled housekeeping may not execute.

Reference: Graypaper [accounts.tex](vendor/graypaper/text/accounts.tex), “Service Privileges.”

## Medium — liveness, migration, and model fidelity

### 11. The 4 KiB head cap is an unverified migration constraint

A 4 KiB limit is not inherently incompatible with every existing parachain, as the previous audit
claimed. It is nevertheless lower than the SDK's supported 1 MiB maximum. Migration needs either an
inventory proving every target chain fits the new cap, an explicit compatibility break, or a larger or
committed head representation.

References: design §3.1; SDK
[polkadot/primitives/src/v9/mod.rs](vendor/polkadot-sdk-cumulus/polkadot/primitives/src/v9/mod.rs) and
[cumulus/pallets/parachain-system/src/lib.rs](vendor/polkadot-sdk-cumulus/cumulus/pallets/parachain-system/src/lib.rs).

### 12. XCMP and D3L messaging are still intentionally TBD

The design explicitly leaves channel-management and message host functions unspecified. A complete
protocol still needs channel lifecycle, routing, sequence/watermark rules, congestion and fees,
acknowledgements, replay protection, retention/recovery, and migration of HRMP/UMP state.

D3L integration must additionally define deterministic 4,104-byte segmentation, export counts,
segment-root/index proofs for imports, and how on-chain headers bind to those segments. Until then,
“full XCMP” and the XCM-dependent collator-set rotation flow are proposals rather than implementable
parts of the service.

References: design §7.1 and §8; Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), `export`, and
[work_packages_and_reports.tex](vendor/graypaper/text/work_packages_and_reports.tex), segment definitions.

### 13. Forced cleanup can name state but cannot enumerate it

The updated `forget(para_id, ...)` and `kv_remove(para_id, ...)` calls let Coretime clean another
parachain's known state. That fixes the old authorization problem, but JAM storage has no prefix iterator
and the layout has no per-parachain index of arbitrary KV keys and solicited preimages.

For a bricked or malicious parachain, Coretime therefore needs an authoritative source for every exact
key/hash/length or a bounded on-service sweeper/index before `parachain_clean_up` can reach its required
baseline-only state.

References: design §3.1, §4.3, and §6.4.

### 14. Validation-code timeout cleanup is still lazy

A timed-out pending upgrade is released only while processing another successful-parent candidate for
that parachain. An idle or dead parachain can therefore retain the pending preimage indefinitely despite
the advertised 24-hour timeout.

The timeout needs an always-accumulate deadline index or must be documented as an adoption deadline that
does not guarantee storage cleanup. The rule should also say whether accumulation delay can invalidate a
candidate refined or guaranteed before the deadline.

References: design §5.1 and §5.2.

### 15. Graypaper work errors have no service-level attribution or recovery protocol

JAM can replace a work item with `WorkExecResult::Error` for out-of-gas, panic, bad exports, oversized
output, or unavailable/oversized service code. The design deliberately skips every such result: no
parachain log, state transition, or durable association with the affected parachain remains.

Some failures must be ignored because no trusted service digest exists, but operational recovery and
any collator accountability still need a specified source of attribution, such as the report's work-item
and authorizer trace. The protocol should classify the error codes, state which are retryable or
slashable, and define who observes and acts on them without trusting a failed Refine result.

References: design §3.3 and §5.1; Graypaper
[reporting_assurance.tex](vendor/graypaper/text/reporting_assurance.tex), `WorkError`.

### 16. The Quint model's refine and service upgrade confuse solicitation with availability

The model now tracks JAM request status in `preimageStatus`, but `codeAvailable` still checks only that a
non-empty `preimageRegistry` referencer set exists. Initial codes are `Unprovided`, yet the model can
refine with them. It also ignores the lookup-anchor timeslot needed for historical availability.

`UpgradeService` repeats the same issue: a registry entry is treated as a present service-code preimage
even when its status is `Unprovided` or `Unrequested`.

References: [quint/state.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state.qnt),
[quint/refine.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine.qnt), and
[quint/accumulate.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate.qnt).

### 17. The modeled encoding-size calculations are not exact

`resultExceedsBudget` assumes a 256-byte authorizer trace, which is the stored-log truncation limit rather
than a Graypaper authorizer-trace limit. It also omits successful-result encoding bytes such as the enum
tag, validation-code length, lookup-anchor timeslot, and container prefixes.

Graypaper limits the exact successful result blobs plus the full trace. The model should either encode
the modeled digest exactly or conservatively include every field and permit the actual trace length as
an input. Separately, `compactLen` treats every integer at or above `2^30` as five bytes even though
SCALE's big-integer mode is variable-width, and `logEntrySize` hard-codes a one-byte vector prefix for
every accumulate-event batch. These make the claimed exact 64 KiB log accounting incorrect as well.

References: [quint/refine.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine.qnt),
[quint/state.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state.qnt),
and Graypaper [reporting_assurance.tex](vendor/graypaper/text/reporting_assurance.tex),
`C_maxreportvarsize`.

### 18. Important Graypaper failure and authorization semantics remain abstract or wrong in the model

The model is useful for the service's internal state machine, but it does not yet validate several
boundaries relevant to the design:

- `TransferOut` is a no-op in message replay and does not model JAM transfer return codes or balance.
- Pending and emitted authorizer queues may contain fewer than JAM's exact 80 hashes.
- The authorization pool transition and report-admission timing are absent.
- Modeled JAM `solicit` and `forget` effects retain only the hash, dropping the length that is part of
  the operation's identity.
- The modeled candidate supplies a validation-code length that is absent from the design's candidate
  payload, avoiding the need to derive and authenticate that value.
- Real assigner/delegator/always-accumulate privileges and host-call failures are absent.
- The actual service balance and `FULL`, `LOW`, `CASH`, panic, out-of-gas, and checkpoint behavior are
  absent.
- D3L and messaging are absent, consistently with the design's TBD.

These are legitimate abstractions, but invariants proved over the model do not cover those properties.

References: [quint/accumulate.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate.qnt),
[quint/invariants.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/invariants.qnt), and
[quint/README.md](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/README.md).

## Low — documentation drift

### 19. The design and Quint README still contain stale or contradictory text

Current examples include:

- the table of contents advertises a missing “Missing JAM / Gray Paper Features” section;
- §7 refers to `UpwardMessage::SetAuthorizerQueue`, which was replaced by `AssignCore`;
- the prose `UpwardMessage::Forget` and `RemoveKV` variants omit the target `para_id` now present in the
  Quint model and required for Coretime cleanup;
- `used_state_balance` is still commented as preimage-only despite also charging baseline and KV state;
- §5.1 first says a Refine error does not prune the log, then says every candidate is pruned before its
  Refine or Accumulate entry is appended;
- the Quint README still cites the removed `testBounceOnFull` behavior, points anchor handling to a
  missing §9, and says the top-level invariants live in `main.qnt` rather than `invariants.qnt`.

These should be corrected upstream so the prose, model, and implementation describe one protocol.

## Current model verification status

With Quint 0.32.0, the current vendored model:

- typechecks;
- passes all 49 scripted tests; and
- passes both the formerly failing `parent_head_continuity` seed (`0x513689`, 10,000 samples, 30 steps)
  and the composite `invariants` check with the same run parameters.

The old reproducible-invariant finding is therefore resolved and has been removed from the gap list.
