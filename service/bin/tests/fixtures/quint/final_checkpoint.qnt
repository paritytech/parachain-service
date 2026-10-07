// Final-checkpoint failures retain all report state and effects, but no commitment.
module final_checkpoint {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  import assignment_inputs.* from "./assignment_inputs"
  import lifecycle_inputs.* from "./lifecycle_inputs"
  import transfer_inputs.* from "./transfer_inputs"

  import validator_key_inputs.* from "./validator_key_inputs"

  import outgoing_inputs.* from "./outgoing_inputs"

  import service_upgrade_inputs.* from "./service_upgrade_inputs"

  import invocation_inputs.* from "./invocation_inputs"

  import fuzz.* from "./fuzz"

  pure def arrival(amount: int, memo: int): IncomingTransfer = {
    source: MkServiceId(7), amount: amount, toSupervisorBalance: false, memo: memo
  }
  def hubWork(mode: int, msgs: List[UpwardMessage]): WorkResult =
    replayWork(svc, AssetHubParaId, svc.paraInfo(AssetHubParaId).headData,
      activeCodeCandidate(svc, AssetHubParaId), mode, now, 0, msgs)

  import gas_inputs.* from "./gas_inputs"

  action schedule = replayBlock(List(replayWork(svc, CoretimeParaId, 0,
    activeCodeCandidate(svc, CoretimeParaId), 0, 0, 0,
    List(AssignCore({ core: 0, queue: List({ authBytes: 10 }),
      jamSlot: 2, newAssigner: None })))), 1)
  action resume = replayInvocation(List(hubWork(0,
    List(SetKV({ key: List(73), value: List(9) })))), now + 1, List(arrival(1000, 3)))

  run retained_effects_works = replayInit.then(schedule).then(finalCheckpointBlock).then(resume)
  run repeated_works = replayInit.then(finalCheckpointBlock).then(finalCheckpointBlock).then(resume)
  run empty_works = replayInit.then(budgetedInvocation(List(), 1, List(), List(), 0)).then(resume)
}
