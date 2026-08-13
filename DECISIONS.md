# Implementation Decisions

Decisions made while implementing the Proof-of-Concept of the
[Parachain Service design](vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/parachain-service-on-jam.md),
in the places where the design under-specifies, contradicts itself, or leaves an
implementation choice open (see [SPEC_GAPS.md](./SPEC_GAPS.md)). Each entry names the gap it
resolves (if tracked) and what should be fed back into the spec.

Format: **D-n** — decision, rationale, spec feedback. Entries are deleted once a GitHub
issue exists for them; numbering is not reused.

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

## D-8: incoming transfers are recorded in one bucket write per block

Relates to SPEC_GAPS #2.

The Quint model records each incoming transfer individually. Since all of a block's
transfer operands arrive at the same timeslot (one bucket), a literal port re-reads and
re-writes the growing bucket per transfer — measured at 551M gas for 1024 same-slot
transfers, 55x the Gray Paper's whole per-report budget `Ga = 10M`
(`accumulate_gas.rs::incoming_transfer_flood_works`). The PoC batches: admission is checked
per transfer in operand order (identical semantics, including the count-based cap), then
all admitted transfers land in a single bucket write. Measured cost drops to 1.63M gas.

**Spec feedback**: the model's per-transfer processing is fine as a spec of *meaning*, but
§5.1 should note that recording must be batched per block (or the §3.1 bucket layout must
be chunked) — a conforming literal implementation cannot fit its own gas budget.

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

## F-10: the 1024-message digest cap has no gas headroom against `Ga`

A reachable worst-case digest (1024 `solicit`s, 36 KiB — fits the report bound) replays at
7.87M gas in the unoptimized PoC, 0.79x the Gray Paper's per-report accumulate budget
`Ga = 10M` (`accumulate_gas.rs`). Out-of-gas mid-replay reverts to the last checkpoint,
un-enacting a candidate Refine already validated. §4.3's `MAX_UPWARD_MESSAGES_PER_DIGEST`
needs deriving from `Ga` (with margin), not picked independently. Pinned by
`solicit_flood_works` / `set_kv_flood_works`.

## F-11: several message types cannot reach the 1024-message cap anyway

The digest is part of the work-report's elective data, capped by Gray Paper
`Wr = 48 KiB`. A `TransferOut` encodes to ~134 B (memo alone is 128 B), so at most ~345 fit
in a report; 1024 of them encode to 137 KiB. The §4.3 cap should be stated per encoded
size (i.e. derived from `Wr`), not as a flat message count.

## F-12: the non-candidate gas budgets need sizing

The design expects the service to be always-accumulate (§2) but never sizes the
`always_acc` privileges allotment that pays for operand-less maintenance work. Measured
worst case: flushing a due `assign` for every core (341 entries, full 80-hash queues) in
one block costs 9.94M gas — ~29k per assign, dominated by the `assign` host call itself
(`accumulate_gas.rs::due_assign_flood_works`); steady state is one core per slot. An
allotment of ~10M covers the avalanche, and it is reserved on top of the block's
accumulation pool, so it does not compete with candidate gas.

Similarly, incoming-transfer recording is paid by the transfers' own `gas_limit` (the JAM
scheduler adds it to the invocation unconditionally), so the service's `min_memo_gas` must
cover the measured ~1.6k-per-transfer recording cost with margin — a token value like the
mock's 100 would be under water.

## F-13: forwarded transfer gas multiplies past `Ga` despite both caps

Sharpens F-10. `Ω_T` charges each replayed `TransferOut`'s forwarded gas to the sender's
meter. Both bounds are individually enforced — per-transfer `MAX_TRANSFER_GAS = Ga/100`
(#17), per-report ~345 transfers under `Wr` (F-11) — but their product exceeds `Ga`: an
Asset Hub digest of 345 transfers to a destination demanding `min_memo_gas =
MAX_TRANSFER_GAS` measures 36.4M gas — 3.64x `Ga`
(`accumulate_gas.rs::transfer_out_hostile_dest_flood_works`) — so in production it
out-of-gasses after ~90 replays and un-enacts the validated candidate. Destinations are
user-chosen, so this is reachable. The replay loop needs a **cumulative**
forwarded-gas budget per digest (a fraction of `Ga`), not only a per-transfer cap — or the
§4.3 caps must be co-derived so `count x per-transfer ≤ margin x Ga`.
Only Asset Hub digests may carry `TransferOut`, and 345 is the most ~134-B transfers that
fit the `Wr = 48 KiB` report bound.
