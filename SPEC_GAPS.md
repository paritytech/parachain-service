# Spec gaps — Parachain Service on JAM vs. Graypaper 0.8.0

## Audit scope

This file tracks unresolved specification issues in the current
[Parachain Service design](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/parachain-service-on-jam.md)
and its [Quint model](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint), checked against
the vendored JAM Graypaper, version [0.8.0](vendor/graypaper/VERSION). See
[VENDOR.md](./VENDOR.md) for the pinned revisions.

The audit below is against the `polkadot-sdk-quint` spec tree checked out at commit
`4a22816d19a943688a3eef82b3fe4446667de812` (branch `bkchr-parachain-service-doc`; see VENDOR.md for
the pin history). Every numbered gap carries an explicit verdict from
{RESOLVED-UPSTREAM, CHANGED-SHAPE, UNCHANGED, RESOLVED-IN-POC, BLOCKED-NEEDS-SPEC-CHANGE} with a
`path:line` citation into that tree. Citations are relative to
`vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/` (e.g. `parachain-service-on-jam.md`,
`quint/refine.qnt`) or `vendor/graypaper/text/` (e.g. `accounts.tex`).

This is deliberately a list of current, actionable gaps. Resolved findings and general implementation
TODOs are not retained here. The Rust implementation is consulted where noted, but its vendored
[PolkaJAM types](vendor/polkajam/crates/jam-types/src/simple.rs) still declare Graypaper version 0.7.2;
those comparisons identify local design/implementation drift rather than establish complete
implementation compatibility with Graypaper 0.8.0.

The design has improved substantially since the previous audit. The remaining issues are concentrated
around the consensus ABI, JAM authorization timing and privileges, economic backing and transfer
reconciliation, upgrade and cleanup liveness, messaging, and fidelity of the formal model.

Gaps are ordered by severity, most impactful first — both across the tiers and within each tier.

> **PoC status.** The Rust implementation now covers the full service (state layout §3.1,
> Refine child host-call ABI §4, the complete Accumulate pipeline §5, state-balance
> accounting §6.1, management §6.2–6.4, and the AURA authorizer §7.1 with real ed25519 +
> Merkle-proof verification (commit b00be73) — messaging §8 remains TBD). Decisions
> and findings are tracked in [DECISIONS.md](./DECISIONS.md) (live entries: D-2, D-4, D-5,
> D-8, D-10, D-11, D-12; F-1, F-8 through F-15); entries retired once an upstream issue
> existed are mapped in DECISIONS.md's Retired-decisions table (D-1, D-3, D-6, D-7, D-9,
> F-4, F-5, F-6). Highlights of this re-audit: #6's child-PVF ABI is pinned by the
> implementation (D-1, `service/src/pvf/executor.rs`); #2's transfer gas and admission are
> reworked per D-8/D-10/D-11/F-14; #4's wire types are `Compact<u64>` (D-3); #9's queue
> expansion is defined by cycle repetition (D-7); #16's availability split is documented
> (F-6). See each entry's verdict below.

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

> **Verdict (audit 2026-08-19):** BLOCKED-NEEDS-SPEC-CHANGE — re-verified against the restored spec:
> the candidate PoV is still an opaque "block data + witness" blob with no authenticated service-state
> input, proof encoding, or verification rule specified; §5.4's "validation inputs" remain undefined.
> Ref: `parachain-service-on-jam.md:438-445` (§3.2 candidate), `parachain-service-on-jam.md:583-607`
> (§4.1 Refine steps), `parachain-service-on-jam.md:641-659` (§4.3 data-access host functions).

References: design §3.2, §4.2–§4.3, §5.2, and §5.4; Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), “Refine Invocation,” and
[accounts.tex](vendor/graypaper/text/accounts.tex), “Historical Preimage Lookups.”

### 2. Transfer gas, admission, and financial reconciliation remain incomplete

The transfer path was substantially reworked in the design's §5.1 (pinned by D-8, D-10, D-11, F-14),
which closes most of the original bullets:

- `transfer_out` now carries its gas explicitly as `deferred: Some((memo, gas))` (§3.3), forwarded to JAM
  `transfer` and charged against the Parachain Service's own accumulate gas, with a per-transfer cap
  (`MAX_TRANSFER_GAS`, F-13). The sender-side cap still has no `TransferError` variant (F-14).
- Incoming-transfer admission is a count-based cap (`MAX_INCOMING_TRANSFERS`) checked per operand in
  order and backed by a tracked queue count (D-8); recording lands in one bucket write per slot.
- The reserved-portion-full case is now explicitly best effort: JAM has already credited the funds, and
  the design states they are kept either way while the unrecorded transfer is dropped (§5.1).
- Zero/tiny transfers are handled by the self-funding rule beyond the reserved portion (§5.1).

The remaining issues are narrower:

- `min_memo_gas` and the reserved queue size are still PROVISIONAL — the design notes they must be
  benchmarked against the real cost of admitting one transfer and derived from it (§5.1, §6.1).
- Asset Hub's burn/refund/retry protocol for failed outbound transfers is still unspecified; only the
  structured `AccumulateLog::TransferFailed` echo exists.
- The vendored GP 0.7.2 host executes only the deferred mode; the supervisor-selector and plain-move
  shapes remain forward-compatibility (D-11).

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — the transfer design was materially reworked upstream
> (§5.1) and the PoC pins the wire/executable shapes (D-8/D-10/D-11/F-14; `service/src/accumulate/transfers.rs:33`,
> `service/src/constants.rs:35,58`), leaving benchmark-derived sizing and Asset Hub recovery policy open.
> Ref: `parachain-service-on-jam.md:725-760` (incoming admission), `parachain-service-on-jam.md:860-887`
> (outgoing transfers), `parachain-service-on-jam.md:521-529` (`TransferOut` wire shape),
> `parachain-service-on-jam.md:1310-1313` (provisional `N`); `pvm_invocations.tex:855-875` (GP `transfer`).

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

> **Verdict (audit 2026-08-19):** UNCHANGED — the design still only says to checkpoint after each
> work-report and does not prove one accepted digest fits its gas allocation.
> Ref: `parachain-service-on-jam.md:885-887` (checkpointing), `parachain-service-on-jam.md:690-693`
> (1024-message cap).

References: design §4.3, §5.1, and §5.3; Graypaper
[accumulation.tex](vendor/graypaper/text/accumulation.tex), “The Accumulation Function,” and
[definitions.tex](vendor/graypaper/text/definitions.tex), `C_reportaccgas`.

### 4. Per-parachain quotas are not backed by the real service balance

`total_state_balance` and `used_state_balance` are private accounting values. The design's §6.1 now
defines their management in detail — Coretime is the sole authority on `total_state_balance`,
`parachain_set_state_balance` applies only when `new_total >= used_state_balance` (rejected with a log
otherwise), and every growth is pre-checked against headroom before the write — but there is still no
invariant connecting:

- the sum of per-parachain reservations;
- Asset Hub's global reservation;
- the service base deposit and `gratis`; and
- the Parachain Service account's actual JAM balance.

Consequently, private headroom can exist while a real `write` or `solicit` returns `FULL`. The funding,
escrow, refund, and insolvency-recovery flows need to be defined. The wire-type mismatch is resolved
implementation-side: the PoC uses `Balance = u64` with `Compact<u64>` wire encodings and compile-time
pins (D-3), while the design's sizing tables and the Quint model still comment `Compact<u128>`
(§6.1, `quint/state_balance.qnt`).

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — §6.1 now specifies the per-para accounting management
> and write-time invariant, and the PoC resolved the wire types to `Compact<u64>` (D-3;
> `service-interface/src/types.rs:27-29`, `service/src/state_balance.rs:8`); the design/model still size
> balances as `Compact<u128>` and the real-balance invariant remains open.
> Ref: `parachain-service-on-jam.md:1144-1172` (§6.1 total-balance management),
> `parachain-service-on-jam.md:1339-1350` (write-time invariant), `parachain-service-on-jam.md:1221`
> (`Compact<u128>` sizing), `quint/state_balance.qnt:19`; `accounts.tex:135-155` (footprint/threshold).

References: design §6.1; Graypaper [accounts.tex](vendor/graypaper/text/accounts.tex), “Account Footprint
and Threshold Balance”; [jam-types `Balance`](vendor/polkajam/crates/jam-types/src/simple.rs).

### 5. Service self-upgrade lacks a safe compatibility and recovery protocol

The prose now specifies the exact check: Accumulate verifies that the new code's preimage is available
for lookup — JAM's `query` reports it as provided, not merely solicited (§5.4) — and the Quint model's
`UpgradeService` replay implements that via `preimageAvailable` (unlike Refine's `codeAvailable`, which
still checks only registry membership; see #16). The PoC mirrors it: `upward.rs` calls `is_available`,
which is the JAM `lookup` host call, so a solicited-but-unprovided preimage is rejected. JAM `upgrade`
itself still performs no well-formedness check, and the PoC does not yet decode the blob as a canonical
`(metadata, code)` service blob before upgrading.

It also still lacks a fallback/recovery authority if the new code cannot execute. Results refined and
guaranteed under the old service code may be accumulated after activation by the new code, but no digest
or storage version and no pipeline-drain/compatibility rule is specified.

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — the "available for lookup" requirement is now
> specified (§5.4), implemented by the model (`preimageAvailable`) and by the PoC (`is_available` →
> JAM `lookup`; `service/src/accumulate/upward.rs:106-117`); the canonical-blob decode and the
> recovery/compatibility authority remain open.
> Ref: `parachain-service-on-jam.md:1015-1044` (§5.4), `quint/accumulate.qnt:301-313`,
> `quint/state.qnt:187-194` (`preimageAvailable`); `accounts.tex:40-43` (Code and Gas).

References: design §5.4; Graypaper [accounts.tex](vendor/graypaper/text/accounts.tex), “Code and Gas,”
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), `upgrade`, and
[reporting_assurance.tex](vendor/graypaper/text/reporting_assurance.tex), “Contextual Validity of Reports.”

## High — security boundaries and deployment blockers

### 6. The implemented child-PVF ABI and the design do not yet agree

There is now a working inner-PVM path, so the old claim that the child PVF was not implementable was
stale. The entry-point shape now conforms to spec §4.2: the design specifies `jam_validate_block() -> ()`
(zero-arg, no return value), and the frameless runtime exports exactly that. The PVF reads its input
(the `ParachainCandidate` payload containing the PoV) via the `work_item_payload(0)` host call (spec
§4.2), and declares its results exclusively through the `set_parent_head_hash` and `set_head` host
calls. The PoC pins the numeric child-host-call ABI in code (D-1, `service/src/pvf/executor.rs`), but
the consensus-facing ABI still needs one canonical, versioned definition.

The specification should pin:

- the accepted program-blob format and exported symbol;
- register, pointer, memory, and return-value conventions;
- every child host-call identifier and encoding;
- child-gas allocation and charging;
- exit/error mapping; and
- ABI version negotiation for runtime upgrades.

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — the entry-point shape is now CONFORMANT to spec §4.2:
> the design and implementation both specify/export `jam_validate_block() -> ()` (zero-arg, no return),
> with input via `work_item_payload(0)` (spec §4.2) and results via `set_parent_head_hash`/`set_head`
> host calls. The PoC pins the numeric child-host-call ABI (D-1, `service/src/pvf/executor.rs:5`).
> The remaining gap is calling-convention-only: §4.3 still does not specify the register/pointer
> conventions, program-blob format, child-gas allocation and charging, exit/error mapping, or ABI
> version negotiation.
> Ref: `parachain-service-on-jam.md:609-632` (§4.2), `parachain-service-on-jam.md:634-693` (§4.3),
> `quint/refine.qnt:24-28` (candidate shape).

References: design §4.2–§4.3; [service/src/pvf/](service/src/pvf/) and
[runtimes/frameless/src/lib.rs](runtimes/frameless/src/lib.rs); Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), “General Functions.”

### 7. The AURA authorizer remains an example, not an executable protocol

The example does not fully constrain the work being authorized. In particular, it does not require work
items to target the Parachain Service or constrain expected service code, gas, imports, and exports, so a
collator can spend para-specific coretime on other JAM work if policy intended otherwise.

It also needs exact rejection rules for zero `slot_duration`/`collator_set_size`, canonical Merkle proof
and non-power-of-two tree semantics, and a canonical domain-separated token-free package encoding. The
proof/signature checks are now implemented and tested (commit b00be73). The round-robin selection still
carries an F-1 anchor-slot caveat: `authorizer/src/is_authorized.rs:50` uses `lookup_anchor_slot`
instead of the anchor timeslot, which is not available in-core (see FIXME there).

> **Verdict (audit 2026-08-19):** RESOLVED-IN-POC — the PoC-side crypto is now real: ed25519
> `verify_strict` (rejecting cofactored/non-canonical signatures and low-order keys) and a real binary
> Merkle-proof walk (blake2b-32 leaf/node hashing, LSB-first sibling ordering, zero-hash padding to the
> next power of two, proof length = ⌈log₂(collator_set_size)⌉), verified by 4 new tests
> (`small_order_key_errors`, `undecodable_collator_key_errors`, `proof_for_wrong_index_errors`,
> `ed25519_known_answer_vector_works`). The §7.1 SPEC-side items remain an open upstream specification
> gap: exact rejection rules for zero `slot_duration`/`collator_set_size`, canonical
> Merkle-proof/non-power-of-two semantics, canonical domain-separated token-free package encoding, and
> the F-1 anchor-slot caveat where round-robin selection uses `lookup_anchor_slot` instead of the
> anchor timeslot (not available in-core).
> Ref: `parachain-service-on-jam.md:1462-1581` (§7.1); `authorizer/src/aura.rs:76` (`check_proof`),
> `authorizer/src/aura.rs:122` (`check_signature`), `authorizer/src/lib.rs:20-26` (`min_stack_size`);
> commit b00be73.

References: design §7.1; [authorizer/src/aura.rs](authorizer/src/aura.rs) and
[authorizer/src/is_authorized.rs](authorizer/src/is_authorized.rs).

### 8. Forced management updates are not fenced from in-flight candidates

Normal candidates and Coretime management messages are processed in operand order. An old candidate may
enact its head and side effects just before a forced head/code replacement; if it appears after the
replacement, the parent or code check rejects it instead. The latter is safe, while the former can leave
side effects from the state being recovered away from.

Forced recovery and deregistration need an epoch/generation fence or a documented core-drain procedure
covering already-guaranteed and not-yet-accumulated work.

> **Verdict (audit 2026-08-19):** UNCHANGED — §6.3/§6.4 describe forced recovery and deregistration but
> no epoch/generation fence or core-drain procedure is specified.
> Ref: `parachain-service-on-jam.md:1376-1441` (§6.3–§6.4).

References: design §5.1 and §6.2–§6.4.

### 9. `assign_core` still does not define a valid JAM queue or on-demand schedule

The updated design correctly combines queue and assigner changes into one operation, and the scheduling
side has advanced: §5.1 defines the always-accumulate flush of due `jam_slot` entries (applied inline
when already due), and §7.2 describes the Coretime-side on-demand policies. The queue-length issue is
now pinned implementation-side: JAM `assign` reads exactly 80 authorizer hashes, and the PoC expands any
non-empty shorter list into an 80-entry `AuthQueue` by cycle repetition (`queue[i mod len]`, D-7),
rejecting only queues longer than 80. The design and Quint model still accept any non-empty list of at
most 80 without stating the expansion rule.

The pool-admission timing remains: applying a queue at `jam_slot` does not reserve that exact slot. At
the end of a block, only the entry at `timeslot mod 80` is admitted to the eight-entry pool; guarantees
in that block were checked against the previous pool. An unused authorization can remain in the pool, and
the queue repeats every 80 slots until replaced.

The on-demand flow therefore still needs the queue construction, lead time, and pool-admission/use timing
stated explicitly in the design, plus a later replacement policy.

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — the shorter-queue expansion is now pinned by the PoC
> (D-7, `service/src/accumulate/assigns.rs:61,69-72`; `service/src/pvf/executor.rs:209-214` rejects
> >80) and §5.1/§7.2 define the flush/scheduling timing; the design still omits the expansion rule and
> the pool-admission/lead-time semantics.
> Ref: `parachain-service-on-jam.md:716-723` (due-assign flush), `parachain-service-on-jam.md:1583-1603`
> (§7.2), `quint/accumulate.qnt:256-295` (AssignCore replay), `quint/state.qnt:85-97` (queue as
> unbounded `List`); `definitions.tex:276` (`Cauthqueuesize = 80`), `pvm_invocations.tex:743-754`
> (JAM `assign`).

References: design §4.3, §5.1, and §7.2; Graypaper
[pvm_invocations.tex](vendor/graypaper/text/pvm_invocations.tex), `assign`, and
[authorization.tex](vendor/graypaper/text/authorization.tex), “Pool and Queue.”

### 10. Required JAM privileges and bootstrap state are not specified

The Parachain Service must be the assigner for every core it manages, the delegator if it calls
`designate`, and an always-accumulate service with enough gas for housekeeping. The manager must also
establish any gratis allowance and recover privileges when required. The design's §2 now states these
three privileged registrations explicitly (always-accumulate membership with a gas allowance, delegator,
and assigner of every managed core), and [GENESIS.md](./GENESIS.md) documents what the PoC requires at
genesis (privilege values, always-accumulate gas sizing, `min_memo_gas` floor) — but the genesis/
bootstrap transition and the authority hand-off between the manager, Coretime chain, Asset Hub, and
Parachain Service are still not designed. Without that state, `assign`/`designate` fail and scheduled
housekeeping may not execute.

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — §2 now explicitly presupposes the three privileged
> registrations, and the PoC's genesis needs are documented in GENESIS.md (Todo 4); the genesis/
> bootstrap transition and the privilege hand-off/recovery protocol remain blocked.
> Ref: `parachain-service-on-jam.md:147-155` (§2 privileged registrations); `accounts.tex:163-180`
> (Service Privileges, incl. `alwaysaccers`).

Reference: Graypaper [accounts.tex](vendor/graypaper/text/accounts.tex), “Service Privileges.”

## Medium — liveness, migration, and model fidelity

### 11. The 4 KiB head cap is an unverified migration constraint

A 4 KiB limit is not inherently incompatible with every existing parachain, as the previous audit
claimed. It is nevertheless lower than the SDK's supported 1 MiB maximum. Migration needs either an
inventory proving every target chain fits the new cap, an explicit compatibility break, or a larger or
committed head representation.

> **Verdict (audit 2026-08-19):** UNCHANGED — §3.1 still caps head data at 4 KiB; no migration
> inventory, compatibility break, or larger/committed representation is specified.
> Ref: `parachain-service-on-jam.md:366-368` (`HeadData = BoundedVec<u8, 4096>`).

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

> **Verdict (audit 2026-08-19):** BLOCKED-NEEDS-SPEC-CHANGE — §8 remains explicitly TBD (the exact
> host functions for HRMP channel management and XCMP message handling are still not specified);
> confirmed correctly out of scope for the PoC. Ref: `parachain-service-on-jam.md:1607-1649` (§8).

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

> **Verdict (audit 2026-08-19):** BLOCKED-NEEDS-SPEC-CHANGE — §6.4 now bounds clean-up by requiring the
> parachain to drain its own extra state first (the service only ever forgets the two validation codes),
> which narrows the enumeration burden, but JAM still has no prefix iterator and no per-parachain index
> exists, so Coretime still needs an authoritative source for the exact keys/hashes/lengths.
> Ref: `parachain-service-on-jam.md:1409-1438` (§6.4).

References: design §3.1, §4.3, and §6.4.

### 14. Validation-code timeout cleanup is still lazy

A timed-out pending upgrade is released only while processing another successful-parent candidate for
that parachain. An idle or dead parachain can therefore retain the pending preimage indefinitely despite
the advertised 24-hour timeout.

The timeout needs an always-accumulate deadline index or must be documented as an adoption deadline that
does not guarantee storage cleanup. The rule should also say whether accumulation delay can invalidate a
candidate refined or guaranteed before the deadline.

> **Verdict (audit 2026-08-19):** BLOCKED-NEEDS-SPEC-CHANGE — §5.1 step 4 and §5.2 Phase 5(b) still reap
> a timed-out upgrade only on the next per-work-package accumulate for that parachain; no
> always-accumulate deadline index is specified.
> Ref: `parachain-service-on-jam.md:785-788` (§5.1 step 4), `parachain-service-on-jam.md:961-966`
> (§5.2 Phase 5b), `quint/code_upgrades.qnt:131-163` (lazy reap).

References: design §5.1 and §5.2.

### 15. Graypaper work errors have no service-level attribution or recovery protocol

JAM can replace a work item with `WorkExecResult::Error` for out-of-gas, panic, bad exports, oversized
output, or unavailable/oversized service code. The design deliberately skips every such result: no
parachain log, state transition, or durable association with the affected parachain remains.

Some failures must be ignored because no trusted service digest exists, but operational recovery and
any collator accountability still need a specified source of attribution, such as the report's work-item
and authorizer trace. The protocol should classify the error codes, state which are retryable or
slashable, and define who observes and acts on them without trusting a failed Refine result.

> **Verdict (audit 2026-08-19):** UNCHANGED — §3.3 still deliberately skips every `WorkExecResult::Error`
> with no log entry, state transition, or attribution; no error classification or recovery protocol is
> specified. Ref: `parachain-service-on-jam.md:573-577` (§3.3 skip note); `pvm_invocations.tex:73-82`
> (Refine `workerror` BAD/BIG).

References: design §3.3 and §5.1; Graypaper
[reporting_assurance.tex](vendor/graypaper/text/reporting_assurance.tex), `WorkError`.

### 16. The Quint model's refine and service upgrade confuse solicitation with availability

The model now tracks JAM request status in `preimageStatus`, but the split is only half fixed:

- `UpgradeService` is now correct: its replay forwards to JAM `upgrade` only when
  `preimageAvailable(codeHash, len)` holds (`Provided`/`Rerequested`), rejecting `Unprovided` and
  `Unrequested` with `ServiceUpgradePreimageMissing` — pinned by a test whose comment notes an
  "`Unprovided` hash would leave the service with no code to run".
- Refine's `codeAvailable` still checks only that a non-empty `preimageRegistry` referencer set exists
  for `(hash, len)`, ignoring the status: initial codes are `Unprovided` (solicited, never provided), yet
  the model can refine with them; and it still ignores the lookup-anchor timeslot needed for historical
  availability.

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — the `UpgradeService` half is RESOLVED-UPSTREAM
> (availability is now checked via `preimageAvailable`), but the Refine `codeAvailable` half remains
> exactly as described, so the entry is not fully RESOLVED-UPSTREAM.
> Ref: `quint/accumulate.qnt:301-313` (UpgradeService), `quint/tests.qnt:1773-1793` (Unprovided-rejection
> test), `quint/refine.qnt:145-151,190` (`codeAvailable`), `quint/main.qnt:44-53` (initial codes solicited
> but `Unprovided`), `quint/state.qnt:187-194` (`preimageAvailable`).

References: [quint/state.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state.qnt),
[quint/refine.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine.qnt), and
[quint/accumulate.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate.qnt).

### 17. The modeled encoding-size calculations are not exact

The model has gained exact per-message and per-log-entry size accounting (`upwardMessageSize`,
`accumulateLogSize`/`refineLogSize`/`logEntrySize`/`parachainLogSize`, all using `compactLen` prefixes),
but `resultExceedsBudget` still assumes a 256-byte authorizer trace (the stored-log truncation limit
rather than a Graypaper authorizer-trace limit) and still omits successful-result encoding bytes: the
`Ok` enum tag, the validation-code length, and the lookup-anchor timeslot. Separately, `compactLen`
treats every integer at or above `2^30` as five bytes even though SCALE's big-integer mode is
variable-width, and `logEntrySize` still hard-codes a one-byte vector prefix for every accumulate-event
batch (the outer `parachain_log` `Vec` prefix now uses `compactLen`, but the batch prefix does not).
These make the claimed exact 64 KiB log accounting incorrect as well.

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — the model's size accounting improved materially
> (exact per-message and per-log-entry sizing with `compactLen` prefixes), but the specific defects the
> entry names persist in `resultExceedsBudget`, `compactLen` (≥2^30 = 5), and the hard-coded accumulate
> batch prefix. Ref: `quint/refine.qnt:65-112` (`upwardMessageSize`, `resultExceedsBudget`),
> `quint/state.qnt:202-208` (`compactLen`), `quint/state.qnt:215-262` (`logEntrySize`,
> `parachainLogSize`).

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

> **Verdict (audit 2026-08-19):** UNCHANGED — the abstractions and their caveat stand. Only two bullets
> are partially addressed: `TransferOut` replay now models the shape-based rejections (D-11) and the
> ghost `solicited` set retains `(para, hash, len)`, but JAM transfer return codes/balance, the exact
> 80-entry queue constraint, pool transition, privileges, service balance, and checkpoint behavior remain
> absent. Ref: `quint/accumulate.qnt:242-255` (TransferOut arm), `quint/state.qnt:85-97` (queue as
> unbounded `List`), `quint/refine.qnt:24-28` (candidate carries `len`), `quint/README.md:38-58` (scope).

References: [quint/accumulate.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate.qnt),
[quint/invariants.qnt](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/invariants.qnt), and
[quint/README.md](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/README.md).

## Low — documentation drift

### 19. The design and Quint README still contain stale or contradictory text

Two drifts were corrected upstream since the last audit (the table of contents no longer advertises a
“Missing JAM / Gray Paper Features” section, and §7 no longer refers to `UpwardMessage::SetAuthorizerQueue`).
The remaining examples include:

- the prose `UpwardMessage::Forget` and `RemoveKV` variants still omit the target `para_id` now present in
  the Quint model and required for Coretime cleanup;
- `used_state_balance` is still commented as preimage-only despite also charging baseline and KV state;
- §5.1 first says a Refine error does not prune the log, then says every candidate is pruned before its
  Refine or Accumulate entry is appended;
- the Quint README still cites the removed `testBounceOnFull` behavior, points anchor handling to a
  missing §9, and says the top-level invariants live in `main.qnt` rather than `invariants.qnt`.

These should be corrected upstream so the prose, model, and implementation describe one protocol.

> **Verdict (audit 2026-08-19):** CHANGED-SHAPE — two of the six drifts are RESOLVED-UPSTREAM (the TOC
> section and the §7 `SetAuthorizerQueue` reference, which is confirmed absent from the whole design
> doc), while the other four persist unchanged.
> Ref: `parachain-service-on-jam.md:5-33` (TOC), `parachain-service-on-jam.md:503-513` (Forget/RemoveKV
> without `para_id`), `quint/messages.qnt:136-142,286-287` (model carries `paraId`),
> `parachain-service-on-jam.md:400-402` (used_state_balance comment), `parachain-service-on-jam.md:774-780`
> vs `812-815` (pruning), `quint/README.md:23,52,62`.

## Current model verification status

With Quint 0.32.0, the current vendored model:

- typechecks;
- passes all 49 scripted tests; and
- passes both the formerly failing `parent_head_continuity` seed (`0x513689`, 10,000 samples, 30 steps)
  and the composite `invariants` check with the same run parameters.

The old reproducible-invariant finding is therefore resolved and has been removed from the gap list.
