# Quint replay coverage TODO

Remaining Rust–Quint replay gaps from the October 2026 audit. Completed work is
removed from this list; current coverage is documented in
[the fixture guide](service/bin/tests/fixtures/quint/README.md).
These are replay coverage limits; separate Rust tests cover some excluded cases.

Prioritize the remaining Accumulate write-site audit.

## 1 Refine executable, byte, and package domains

- [ ] TODO: Extend sampled finite-script gas cutoffs to arbitrary PVF execution
  and broader instruction domains with independent expectations. Zero-entry,
  infinite-child, and sampled finite-script exhaustion now run through combined
  Refine → Accumulate campaigns, with funded controls and subsequent retries.
- [ ] TODO: Extend package inputs beyond the adapter's one-or-two-item domain
  and identical code references, including invalid item counts and heterogeneous
  references. Resolve model-to-Rust error representation gaps explicitly.

## 2 Refine historical lookup lifecycle

- [ ] TODO: Exercise actual historical availability across provision, forgetting,
  code upgrades, and lookup anchors instead of an isolated availability input.

## 3 Accumulate upgrade migrations and transfer delivery

- [ ] TODO: Upgrade between different implementations/storage layouts; both
  replay upgrade blobs currently execute the same implementation.
- [ ] TODO: Execute destination Accumulate after deferred delivery, including
  self-transfer queue entries, rather than only crediting destination balances.

## 4 Successful supervision operations — requires host support

- [ ] TODO: Add mutable supervision to the host, then replay successful foreign
  preimage operations, storage removal, supervisor handoffs, and ejection.
- [ ] TODO: Add supervisor balances and replay successful supervised transfers
  and incoming destination-balance selectors. Keep unsupported traces explicitly
  rejected until the host supports them.

## 5 Generated domains and combined coverage

- [ ] TODO: Generate larger sustained batches and arbitrary byte distributions;
  keep dedicated exact-boundary fixtures and collision-free model mappings.
- [ ] TODO: Make focused Refine boundary generation available to continuous
  campaigns, with assertions for required outcomes.
- [ ] TODO: Extend combined Refine → Accumulate campaigns to the dedicated
  storage-failure/recovery scenarios that currently inject work results directly.
- [ ] TODO: Broaden sampled Accumulate gas cutoffs and combinations with report
  rejection and natural traps. Current sampling is not exhaustive across every
  instruction; preserve independent checkpoint expectations. Due-flush host
  boundaries now have injected gas-fault coverage; natural exhaustion during
  funded flushes and generated fault/rejection combinations remain uncovered.
- [ ] TODO: Extend the authorization replay beyond its fixed single-collator
  tree to collator rotation, deeper Merkle proofs, and authorization gas exhaustion.
