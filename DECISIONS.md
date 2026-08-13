# Implementation Decisions

Decisions made while implementing the Proof-of-Concept of the
[Parachain Service design](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/parachain-service-on-jam.md),
in the places where the design under-specifies, contradicts itself, or leaves an
implementation choice open (see [SPEC_GAPS.md](./SPEC_GAPS.md)). Each entry names the gap it
resolves (if tracked) and what should be fed back into the spec.

Format: **D-n** — decision, rationale, spec feedback.

## D-1: PVF result ABI is host-call based; PoV input stays register-based

Relates to SPEC_GAPS #6 (child-PVF ABI not pinned).

The PVF declares its results exclusively through child host calls, per design §4.2:
`set_parent_head_hash(hash)` and `set_head(new_head)` are each **mandatory exactly once**;
violating that fails Refine with `RefineLog::MissingHeadDeclaration`. The PVF returns no
values through registers.

Deviation from the design's literal `jam_validate_block() -> ()`: the PoV is still passed
*in* through registers as `jam_validate_block(pov_ptr: u32, pov_len: u32)` rather than
fetched by the PVF via a forwarded `work_item_payload` host call. Rationale: the parent
already holds the payload; forwarding it through a fetch round-trip adds plumbing without
changing trust or determinism.

**Spec feedback**: pin the entry-point signature as `jam_validate_block(pov_ptr, pov_len)`,
document register/stack conventions, and state that head declarations are host-call-only.

## D-2: Asset Hub / Coretime ParaIds are compile-time constants

Relates to SPEC_GAPS #10 (privileges and bootstrap state unspecified).

Refine must enforce restricted host functions (§4.3) but is stateless, so it cannot learn
the privileged ParaIds from service storage. The PoC hard-codes
`ASSET_HUB_PARA_ID` and `CORETIME_PARA_ID` in `service-interface`.

Alternative considered: carry a per-item role (Normal/AssetHub/Coretime) in the enforced
authorizer-config prefix (§3.2), which is Coretime-committed via the `assign` flow — no
hardcoding, governable, but a spec change to the prefix shape and a policy obligation on
Coretime to never mark a non-privileged para as privileged.

**Spec feedback**: the design must state where the privileged-para identity lives. Both
options should be listed; constants require a service self-upgrade to migrate Asset Hub or
Coretime to a new ParaId.

## D-3: State-balance accounting uses the exact §6.1 formulas, with `Balance = u64`

Relates to SPEC_GAPS #4 (quotas not backed by real balance; wire types wrong).

The PoC implements §6.1 literally: preimage footprint `187 + len` per referencer,
`kv_entry_footprint(k, v) = 49 + compactLen(k) + |k| + compactLen(v) + |v|`,
`baseline_footprint = 69 847`, the Asset Hub global reservation, delta-charging on
overwrite, and the write-time invariant. This deliberately exercises the spec's arithmetic
so errors surface.

Wire-type correction: JAM `Balance` is `u64`, so all balance fields use `u64`
(SCALE `Compact<u64>` on the wire where the design says `Compact<Balance>`), not the
design's `Compact<u128>`. Worst-case compact size drops from 17 B to 9 B; the §6.1 sizing
tables need re-deriving in the spec.

**Spec feedback**: fix the `Compact<u128>` assumption throughout §3.1/§6.1 and recompute
`baseline_footprint` (the PoC keeps the spec's published constants where they are inputs,
and flags mismatches in tests). The missing link to the real JAM account balance remains
open (gap #4).

## D-4: AURA authorizer implements full logic with stubbed crypto

Relates to SPEC_GAPS #7.

The authorizer implements the complete §7.1 pipeline — config/token decoding, anchor-slot
round-robin collator-index computation, Merkle-proof structure, and domain-separated
signing-payload assembly — but the ed25519 signature check and Merkle-proof verification
accept any well-formed value, behind `FIXME` markers. Rejection rules for zero
`slot_duration` / `collator_set_size` are enforced.

**Spec feedback**: unaffected; real crypto is an implementation milestone, not a spec
question. The §7.1 canonical proof/encoding rules still need specifying (gap #7).

## D-5: `hash(head_data)` is blake2b-256

New finding (not previously tracked). The design's parent-head check (§5.1 step 3) compares
`parent_head_hash` against `hash(ParaInfo.head_data)` but never names the hash function; the
PVF (via `set_parent_head_hash`) and the service must agree on it. We pin **blake2b-256**,
matching JAM's own preimage hashing (`jam_std_common::hash_raw`).

**Spec feedback**: name the function in §5.1/§4.3.

## D-6: TransferOut replay looks up the destination's `min_memo_gas`, capped by `MAX_TRANSFER_GAS`

Relates to SPEC_GAPS #2 (transfer gas unspecified).

Gray Paper `Ω_T` deducts the transfer's full `gas_limit` from the **sender's** accumulate
gas meter on success (`g = C_gasT + l`), and `min_memo_gas` is chosen by the destination
service itself. Passing `service_info(dest).min_memo_gas` uncapped therefore lets a hostile
destination burn arbitrary amounts of the Parachain Service's single accumulate invocation
(losing everything since the last checkpoint and silently dropping all later operands in
the block).

Replay therefore: reads `service_info(dest).min_memo_gas` at replay time; if it exceeds the
service constant `MAX_TRANSFER_GAS` (or the destination does not exist), the JAM `transfer`
is **not** called and `AccumulateLog::TransferFailed { memo_hash }` is appended. Otherwise
the transfer is sent with exactly `min_memo_gas`; a JAM-level failure is also logged as
`TransferFailed`. A destination raising `min_memo_gas` above the cap can only block
transfers to itself (funds never leave the service on failure).

`MAX_TRANSFER_GAS` needs a real benchmark before production (with SPEC_GAPS #2/#3).

**Spec feedback**: specify the lookup + cap in §4.3/§5.1 step 7, and note that AH cannot
supply the value itself (Refine has no `service_info` host call, and AH state cannot read
JAM service metadata trustlessly — gap #1).

## D-7: `assign_core` queues shorter than 80 are cycle-repeated

Relates to SPEC_GAPS #9.

JAM `assign` takes exactly 80 authorizer hashes; the design accepts 1..=80. On replay the
service expands a shorter queue as `queue[i mod len]` for `i in 0..80`. The steady-state
AURA case (one hash for every slot) is exactly the 1-element cycle.

**Spec feedback**: define this expansion in §4.3/§7.1 (or require exactly 80 and drop the
shorter form).

---

# Implementation Findings

Spec issues surfaced while implementing the PoC that did not need a user decision but must
flow back into the design doc / Quint model. Numbered **F-n**; `TODO`/`FIXME` markers in the
code reference the same issues.

## F-1: §7.1 needs the anchor timeslot, but the refinement context does not carry it

The AURA authorizer computes `collator_index` from "the anchor timeslot read from the
refinement context" (§7.1 step 3). The Gray Paper's `RefineContext` exposes only the
**lookup-anchor** slot; the anchor's own slot is not available in-core. The PoC uses
`lookup_anchor_slot` behind a FIXME, which would let a collator pick any lookup anchor
mapping to its own index. The design needs either a JAM change (anchor slot in the context)
or a different slot source. Extends SPEC_GAPS #7.

## F-2: §4.3's `import_segments() -> Vec<SegmentMeta>` has no host-call backing

jam-pvm-common exposes only per-index `import(index)`; there is no segment-metadata list.
The §4.3 data-access table needs correcting (segment counts are available via
`work_item_summary.import_count`). Extends SPEC_GAPS #6.

## F-3: no error codes for oversized `set_head` / oversized `assign_core` queues

A `set_head` beyond the 4 KiB `HeadData` bound and an `assign_core` queue longer than 80
have no specified `RefineLog`. The PoC adds `RefineLog::HeadDataTooLarge` and treats the
oversized queue as a malformed PVF (`ValidationFailed`). §4.2/§4.3 should specify both.

## F-4: the queued-transfer count needs a counter in state

The §5.1 admission rule ("while the queue holds fewer than `MAX_INCOMING_TRANSFERS`")
counts transfers, but JAM storage has no prefix iteration, so the count cannot be recovered
from the buckets. The PoC adds `count: u32` to `IncomingTransferChain` (§3.1 layout
change). Resolves the "unbounded scan or unspecified counter" bullet of SPEC_GAPS #2.

## F-5: §6.1 sizing tables need re-deriving for the real wire types

With `Balance = u64` (D-3) and JAM's `CoreIndex = u16` (the table assumes 4 B):
`baseline_footprint = 69 831` (not 69 847), a worst-case transfer bucket costs 196 (not
204), and the chain pointer grows 4 B for the F-4 counter. The PoC's `state_balance.rs`
unit tests pin the recomputed values. Extends SPEC_GAPS #4.

## F-6: `UpgradeService` verifies actual preimage availability, not registry membership

§5.4 says "verify the preimage is present"; the Quint model checks only a non-empty
referencer set (SPEC_GAPS #16). The PoC checks `is_available(code_hash)` — the preimage
must actually be provided. The prose should say exactly this. (JAM `upgrade` still does not
validate the blob shape — SPEC_GAPS #5 remains.)

## F-7: no log events for failed JAM `assign` / `designate` host calls

A JAM-level `assign` failure (bad core, or the service lost assigner-ship after a
hand-off) and a `designate` failure (service is not the delegator) have no specified
`AccumulateLog`. The PoC logs `DesignateRejected` for the latter and only a debug message
for the former. Extends SPEC_GAPS #9/#10.

## F-8: model fidelity nits found while porting

- The accumulate-side defense-in-depth restriction re-check logs `InvalidCodeHashAcc` in
  the model — misleading; the PoC rejects silently. Needs its own variant or a prose rule.
- The model compares active/pending code identity by **hash only** in
  `requestCodeUpgrade` / `parachainSetValidationCode` although the registry is keyed by
  `(hash, len)` — a same-hash-different-length preimage slips through. The PoC matches the
  model behind TODOs.
- `validator_keys.qnt`'s module doc claims partial appends are balance-charged; the code
  (correctly, per §6.1's Asset Hub baseline pre-provisioning) never charges. Doc drift.
- The model's `ValCount` as a *set* of valid lengths misstates JAM: `designate` takes a
  `FixedVec` of exactly the protocol's validator count. The §5.3 length check is equality,
  not set membership.
- `state.qnt`'s `kvEntryFootprint` comment says map tag `0x07`; the §3.1 table says
  `key_value_storage = 0x08`.

## F-9: AURA config gains a target-service field

To close SPEC_GAPS #7's "does not require work items to target the Parachain Service", the
PoC adds `parachain_service: ServiceId` to the AURA `AuthorizerConfig` and rejects packages
whose items target any other service. §7.1's config struct needs the field.
