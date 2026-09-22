// Preserve the actual operands, including transfers the model drops. Expected
// state comes exclusively from the pinned model's applyIncomingTransfers.
module transfer_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  var replayIncoming: List[IncomingTransfer]
  action noTransfers = replayIncoming' = List()
  action recordIncoming(transfers: List[IncomingTransfer]): bool = all {
    replayIncoming' = transfers,
    svc' = applyIncomingTransfers(svc, transfers),
    now' = now,
    jamStagingSet' = jamStagingSet,
    solicitedSet' = solicitedSet,
    prevSvc' = svc,
    lastStepWorkResults' = List(),
    lastStepAssigns' = List(),
    lastHeadRoot' = None,
    foreignServices' = foreignServices,
    logPrunedBelow' = logPrunedBelow
  }
}
