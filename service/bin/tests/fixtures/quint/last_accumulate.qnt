// Checkpointed initial-flush behavior across repeated invocations.
module last_accumulate {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import fuzz.* from "./fuzz"
  import gas_inputs.* from "./gas_inputs"
  import invocation_inputs.* from "./invocation_inputs"
  import transfer_inputs.* from "./transfer_inputs"
  import validator_key_inputs.* from "./validator_key_inputs"
  import outgoing_inputs.* from "./outgoing_inputs"
  import service_upgrade_inputs.* from "./service_upgrade_inputs"

  action start = replayInit
  action schedule =
    replayBlock(List(replayWork(svc, CoretimeParaId, 0,
      activeCodeCandidate(svc, CoretimeParaId), 0, 0, 0,
      List(AssignCore({ core: 0, queue: List({ authBytes: 10 }),
        jamSlot: 2, newAssigner: None })))), 1)
  action stopped =
    invocationWithRefine(List(replayWork(svc, AssetHubParaId,
      svc.paraInfo(AssetHubParaId).headData, activeCodeCandidate(svc, AssetHubParaId),
      0, now, 0, List(TransferOut(transfer(0, 1, 0, ExhaustPoolGas))))), 2, List(), List(UnlimitedGas), 0, false, None)
  action sameSlot =
    invocationWithRefine(List(replayWork(svc, CoretimeParaId,
      svc.paraInfo(CoretimeParaId).headData, activeCodeCandidate(svc, CoretimeParaId),
      0, now, 0, List(AssignCore({ core: 0, queue: List({ authBytes: 20 }),
        jamSlot: 2, newAssigner: None })))), 2, List(), List(UnlimitedGas), -1, false, None)
  action nextSlot = invocationWithRefine(List(), 3, List(), List(), -1, false, None)
  action preCheckpoint = preCheckpointInvocation(List(), 2, List())
  action incomingTransfer = all {
    recordIncoming(List({ source: MkServiceId(7), amount: 1000,
      toSupervisorBalance: false, memo: 0 })),
    noGas, noDesignation, noOutgoing, keepUpgradeGas
  }
  action slotZero = invocationWithRefine(List(), 0, List(), List(), -1, false, None)
  run slot_zero_works = start.then(slotZero).then(slotZero)
  run same_slot_works = start.then(schedule).then(stopped).then(sameSlot).then(nextSlot)
  run incoming_same_slot_works = start.then(schedule).then(stopped).then(incomingTransfer)
  run incoming_after_rollback_works = start.then(schedule).then(preCheckpoint).then(incomingTransfer)
}
