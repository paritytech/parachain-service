# Parachain Service Genesis Requirements

The Parachain Service requires specific JAM privileges and gas allotments to operate at genesis.
This document specifies the concrete values the PoC currently requires; a production deployment
must establish these through the JAM manager's bootstrap flow.

## Required Privileges

The service must hold the following privilege roles (from `service/bin/src/mock.rs:188-194`):

- **bless**: the service's own `ServiceId` (required to alter privileges and bestow storage credits)
- **assign**: a queue containing the service's own `ServiceId` (required to assign authorizers to cores)
- **designate**: the service's own `ServiceId` (required to set staging validator keys)
- **register**: the service's own `ServiceId` (required to register service IDs in the protected range)
- **always_acc**: the service's own `ServiceId` registered with a gas allotment (see below)

A real genesis must register the Parachain Service's own ID in the `always_acc` map with a sufficient
gas allotment; the mock's `Default::default()` (empty map) is insufficient for production.

## Always-accumulate gas allotment

The service performs housekeeping work without candidate operands (e.g., flushing due `assign` calls).
The worst case is flushing a due `assign` for every core (341 entries, full 80-hash queues) in one block,
measured at `DUE_ASSIGN_FLOOD = 9_943_233` gas (from `service/bin/tests/accumulate_gas.rs:70`).

An allotment of approximately 10M gas covers this avalanche and is reserved on top of the block's
accumulation pool, so it does not compete with candidate gas.

## min_memo_gas floor

Incoming transfers are recorded in the service's state, charged to the transfers' own `gas_limit`.
The measured cost per transfer is `DEST_HANDLER_PER_TRANSFER = 1_665` gas
(from `service/bin/tests/accumulate_gas.rs:72`).

The service's `min_memo_gas` privilege must cover this recording cost with margin; a token value
like the mock's 100 would be insufficient.

## Gratis allowance

The vendored `jam_std_common::Privileges` type (GP 0.7.2) does not model a gratis-related field.
This aspect is not modeled by the vendored GP-0.7.2 privilege type and remains unspecified upstream.

## Not specified upstream

SPEC_GAPS.md entry #10 identifies five open items that require JAM/spec-owner design decisions:

- Assigner-per-core: which service holds the `assign` privilege for each core
- Delegator identity: which service can call `designate` for validator-key changes
- Always-accumulate gas sizing: the allotment required for housekeeping (partially addressed above)
- Gratis allowance: whether and how the service receives free gas for certain operations
- Privilege recovery and hand-off: the protocol for transferring privileges between the manager,
  Coretime chain, Asset Hub, and Parachain Service across epochs

See [SPEC_GAPS.md#10](./SPEC_GAPS.md#10-required-jam-privileges-and-bootstrap-state-are-not-specified)
for the full context.
