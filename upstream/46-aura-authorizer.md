# Upstream the AURA authorizer

Tracking issue: [paritytech/parachain-service#46](https://github.com/paritytech/parachain-service/issues/46).
Related: [#33, runtime API for authorizer hash/config and token construction](https://github.com/paritytech/parachain-service/issues/33).

The AURA authorizer currently lives in this repository, but SDK collators and runtimes
need the same config encoding, collator proofs, and signing payload. Move the shared
protocol and both verifier programs into the Polkadot SDK companion, then consume them
here and delete the local implementations. The SDK must not depend on this repository.

This document is a plan; it does not authorize or implement protocol changes. Package
names and SDK destinations below are proposals to settle with upstream maintainers.

## Intended boundary

Use a lightweight shared primitives library, a guest authorization library, and small
ed25519 and sr25519 verifier programs. Keep host calls and guest entry points out of the
primitives dependency graph. Avoid introducing more crates unless the build requires it.

| Current source | Destination or treatment |
| --- | --- |
| `authorizer/src/aura.rs` | Shared config/token types, signature interface, signing hash, collator selection, and Merkle construction/verification in SDK primitives. |
| `authorizer/src/is_authorized.rs` | Authorization decision and JAM host adapter in the SDK guest library. Keep the decision callable without a host for testing. |
| `authorizer/src/lib.rs` | Guest entry-point/error adapter in the SDK guest library. |
| `authorizer-ed25519/`, `authorizer-sr25519/` | SDK verifier programs, including stack settings and JAM build metadata. |
| `core/src/authorization.rs` | Shared trace/key/signature types in SDK primitives, after resolving development-only control semantics. |
| `core/src/types.rs` | `ParaId` and `ValidationCodeHash` now re-export the SDK's `polkadot_parachain_primitives::primitives` types. Upstream authorizer code can depend on that SDK type directly; do not move unrelated service types. |
| `core/src/authorizer.rs` | Move the reusable authorizer-hash helper and encoding test if useful; reuse `jam_types::Authorizer::hash` rather than adding another algorithm. Keep null-authorizer deployment constants and phase-specific helpers local. |
| `authorizer/tests/` | Move protocol and signature compatibility tests with the implementation. Separate service-specific tests from standalone authorizer tests. |

Keep service execution, chain-spec assembly, parasim commands, development-key handling,
and service integration tests here. They should import the upstream implementation.
Generic config wrapping, hash derivation, and token assembly used by both SDK collators
and local tools should have one shared implementation; do not upstream the CLI around them.

## 1. Settle behavior before extraction

- [ ] Decide how to remove or isolate `SUDO_KEY`. It currently bypasses signature,
  membership, work-item count, and slot checks. The production authorizer must not admit
  unsigned sentinel tokens. Account for parasim's parked-core/reassignment workflow;
  if it still needs the bypass, use a distinct development authorizer with a distinct
  code hash and reuse the common implementation.
- [ ] Decide the resulting `AuthTrace` format and treatment of its `sudo` field. Coordinate
  every producer and consumer; this is a wire-format decision, not cosmetic cleanup.
- [ ] Resolve the design's anchor-timeslot requirement against the implementation's
  `lookup_anchor_slot`. The current FIXME notes that a collator can choose a lookup anchor
  selecting its own index. Agree on the intended scheduling guarantee and record the
  design correction or implementation change before presenting this as production AURA.
- [ ] Specify the configured target-service check in the design. Preserve the protection
  against spending parachain-specific coretime on another JAM service.
- [ ] Document the exact signing and proof protocol: signed fields, domain separator,
  leaf/node hashes, sibling order, padding, and proof depth. Keep SCALE config/token/trace
  encoding distinct from JAM encoding of signed package fields. The config's leading
  `Vec<ParaId>` is consumed independently by service Refine.

Any intentional protocol change needs its own reviewed change and migration notes.
Otherwise preserve encoded bytes and authorization behavior through extraction. Do not
advance the Quint/design submodule outside the repository's one-commit-at-a-time workflow.

## 2. Focused cleanup

Local extraction: `jp-aura-authorizer/` now owns config/token/trace types, the signature
interface, collator selection, Merkle construction/verification, and the signing hash.
It uses SDK primitives directly, supports feature-based `no_std`, and has no dependency
on service-core or JAM host calls. Service and chain-spec consume it directly; old
`authorizer::aura` and `core::authorization` paths re-export the shared definitions.
The development sentinel and bypass remain local; `AuthTrace.sudo` retains its wire
format pending the separate protocol decision. Collator-tree tests moved with the code,
with fixed SCALE and padded-tree vectors added.

Cleanup: split the shared library into config, token, Merkle, signing, and trace modules
while retaining the root exports. Shortened API comments and documented helper preconditions;
removed the authorizer's self-dependency and self-referential features. Deleted the unwired
trace-size test already covered by `authorizer/tests/aura.rs`, and corrected its stale comment
and test name. Protocol validation and development-bypass changes remain separate work.
Cleanup validation: 670 tests pass, none failed or ignored, including gas and generated
replay tests. All four guests and standalone `std`/`no_std` checks pass; gas pins are unchanged.

Extraction validation: all four PVM guests build, workspace checks and standalone
`std`/`no_std` checks pass, and the full suite passes (670 tests, none failed
or ignored). Gas pins now reflect the rebuilt programs: ed25519 authorization
4,018,923; solicit 65,465,419; KV writes 53,462,999; both transfer workloads
4,904,567. The other pins and the report gas-margin assertion remain unchanged.


- [x] Remove service-core from the authorizer's production dependencies. The extracted
  library supports feature-based `no_std`; service-core remains a test dependency.
- [ ] Simplify manifest features, including self-referential feature forwarding and
  service-specific test dependencies. Keep keystore/signing dependencies in host tests
  or host consumers, not verifier guests.
- [ ] Validate public helper inputs or make helpers internal: collator selection currently
  assumes nonzero set size/duration, and proof checking assumes an in-range index.
  Define the public tree builder's behavior for an empty set.
- [ ] Replace stale or excessive comments with short API contracts and protocol rationale.
  Remove references to nonexistent `*-bin` build scripts and obsolete trace fields.
  Keep explanations of consensus behavior, encoding, stack requirements, and size limits.
- [ ] Treat invalid authorization as expected rejection rather than describing every failure
  as a software bug, while preserving JAM's required rejection behavior.
- [ ] Consolidate overlapping/unwired tests, including `authorizer/src/tests/aura.rs`, and
  use the repository's `[<context>|trivial]_[errors|works]` naming convention.

Keep the existing separation between scheme-independent logic and signature verification.
Do not replace the Merkle implementation merely to reuse an SDK utility: the SDK's
`binary-merkle-tree` promotes odd leaves, while this protocol zero-pads to a power of two.
Any replacement must preserve the agreed protocol or be reviewed as a protocol change.

## 3. Add the SDK implementation

- [ ] Agree on package names and location with SDK maintainers, with the JAM/Cumulus code
  as the likely home. Keep the related runtime API in #33 a separate deliverable unless
  maintainers explicitly combine the work.
- [ ] Add shared primitives, guest authorization, and both verifier programs with no
  dependency back to `parachain-service`.
- [ ] Align JAM/PolkaVM dependency sources and versions. Avoid duplicate crate identities
  when JAM types cross repository boundaries.
- [ ] Provide a supported way for consumers to build or obtain both authorizer blobs.
  Carry over the size-oriented build profile, hashing feature choices, and guest stack
  configuration; SDK workspace profiles must support the same build constraints.
- [ ] Move protocol and cross-scheme keystore tests upstream, replacing service payload
  fixtures with opaque payloads where service behavior is irrelevant.

## 4. Consume upstream and remove duplication

- [ ] Update the companion pin and workspace dependencies to the upstream packages.
- [ ] Delete `authorizer/`, `authorizer-ed25519/`, and `authorizer-sr25519/` after their
  consumers have migrated. Remove obsolete workspace members and dependency entries.
- [ ] Replace local shared type/helper definitions with upstream imports or thin re-exports.
  In `service/src/refine.rs`, import the trace from shared primitives rather than the guest
  authorizer implementation.
- [ ] Update `chain-spec/`, `service/bin/src/mock.rs`, `service/bin/src/lib.rs`,
  `service/bin/tests/is_authorized.rs`, `testing/parasim-service/`, and
  `tools/parasim-tool/src/aura.rs` to consume the upstream interfaces.
- [ ] Update parasim's blob discovery in `tools/parasim-tool/src/main.rs`; it currently
  assumes local authorizer packages/artifacts. Verify the workflow from a fresh checkout.
- [ ] Preserve useful measurements from `authorizer-sr25519-experiment/` in documentation
  or history, then remove its duplicate implementation from the active tree.
- [ ] Search for remaining local proof, signing-payload, config, and trace implementations.
  Keep independent test vectors, but no second production implementation.

## Validation and completion

- [ ] Fixed vectors pin SCALE config/token/trace bytes, the JAM-encoded signing payload,
  signing hash, authorizer hash, and Merkle roots/proofs, including non-power-of-two sets.
  Builder/verifier round trips alone do not establish compatibility.
- [ ] Host tests cover both keystore signature schemes, wrong-scheme signatures, altered
  packages/anchors/configs, invalid membership/proof lengths, invalid set size/duration,
  wrong target service, and work-item/para-count mismatches.
- [ ] Both actual PVM verifier blobs accept valid packages and reject invalid ones.
  Check blob size against the active JAM authorizer limit and execution against the applicable gas
  budget, including representative large proofs. Preserve stack requirements.
- [ ] Production verifier tests reject the former unsigned sentinel bypass. Test any
  retained development authorizer separately.
- [ ] SDK host/runtime/guest builds pass with the intended feature combinations. This
  repository's workspace checks, authorizer/service integration tests, and full replay
  suite (including the opt-in generated checks) pass against the new pin.
- [ ] Parasim can discover/build the upstream blobs and complete its supported assignment
  and submission workflows; existing configs/queues are either compatible or have an
  explicit regeneration/redeployment procedure. Rebuilt code can change authorizer hashes
  even when wire formats remain unchanged.

Land the work as reviewable steps: behavior decisions and cleanup, SDK addition, then
the local dependency switch and deletion. Close #46 when this repository consumes the
SDK implementation and no longer maintains duplicate authorizer code.

## Other local duplication to revisit

These are separate follow-ups, not prerequisites for extracting the authorizer.

- [x] Replace local `ParaId` and `ValidationCodeHash` with SDK primitives. Keep
  the raw-byte conversion for validation-code hashes at JAM API/storage boundaries.
- [x] Reuse SDK `sp-crypto-hashing` for the Blake2/Keccak wrappers in
  `core/src/lib.rs`, `core/src/authorizer.rs`, `service/src/hashing.rs`, and
  `runtimes/frameless/src/lib.rs`. These now re-export the SDK functions; the
  affected crates no longer directly depend on `blake2b_simd`/`tiny-keccak`.
  No new packages were added to the lockfile. All four guest builds pass.
  Service blob: 186,922 → 187,900 bytes; frameless: 60,220 → 62,512 bytes;
  both authorizer blob sizes are unchanged.
  Workspace checks and 538 functional tests pass (23 opt-in replay tests ignored).
  The eight gas measurement pins were refreshed after this swap; measured gas is
  unchanged or slightly lower than before the hashing swap. The gas-margin test
  now executes reports sized to the service gas gate (currently 20 messages),
  declares the actual single-report limit, verifies the reports/messages applied,
  and checks 20% execution headroom. The KV case fills the report's byte limit.
  The larger stress benchmarks retain their exact cost pins; they use unrestricted
  declared gas and are not evidence that one network work report can carry that load.
  All nine gas tests pass with none ignored.
- [x] Reuse the same SDK `blake2_256` re-export in `authorizer/src/aura.rs`
  for Merkle hashing and the signing hash. Retain the direct `blake2b_simd`
  dependency solely to enable `uninline_portable` for the SDK implementation.
  Both authorizer guests build: ed25519 shrinks from 67,570 to 65,033 bytes;
  sr25519 from 60,574 to 57,950 bytes. Ed25519 authorization gas changes from
  4,004,134 to 4,048,126; its exact measurement pin is updated.
  The chain-spec validation-code hash test now builds this repository's frameless
  PVF instead of requiring a fixture in an external sibling checkout.
  Full workspace tests, including opt-in replay/generated tests and doc tests:
  668 passed, none failed or ignored. Workspace checks and all four guest builds pass.
- [x] Re-export Polkajam's primitive aliases for `Timeslot` (`Slot`), `ServiceId`,
  `CoreIndex`, and `Balance` instead of repeating their underlying integer types.

Similar names do not always mean interchangeable types:

- Keep the local bounded `HeadData`: the SDK's vector does not enforce our 4 KiB limit.
- Keep scheme-neutral collator key/signature bytes until the shared authorizer API is
  settled; SDK collator types select a particular signature scheme.
- JAM hash/memo/validator-key wrappers use JAM codec traits; our wire types need SCALE.
  Do not substitute wrappers based solely on matching byte layouts.
- Preserve the authorizer Merkle padding rule; SDK `binary-merkle-tree` handles odd
  leaves differently, as described above.
