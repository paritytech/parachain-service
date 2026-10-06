// Input generator for the supported replay profile. No expected states
// or transition sequences are prescribed here; semantics come from the pin.
module gas {
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
    from: MkServiceId(7), amount: amount, toSupervisorBalance: false, memo: memo
  }
  def hubWork(mode: int, msgs: List[UpwardMessage]): WorkResult =
    replayWork(svc, AssetHubParaId, svc.paraInfo(AssetHubParaId).headData,
      activeCodeCandidate(svc, AssetHubParaId), mode, now, 0, msgs)

  import gas_inputs.* from "./gas_inputs"

  action boundary(mode: int, msgs: List[UpwardMessage]): bool = {
    val wr = hubWork(0, msgs)
    budgetedInvocation(List(wr), now + 1, List(arrival(1000, mode)),
      List(budget(wr, mode)), -1)
  }
  run boundaries_works = replayInit
    .then(boundary(0, List()))
    .then(boundary(1, List()))
    .then(boundary(2, List()))
    .then(boundary(3, List(SetKV({ key: List(9), value: List(1) }))))
    .then(boundary(1, List(TransferOut(transfer(0, 100, 1, 100)))))
    .then(boundary(2, List(TransferOut(transfer(0, 100, 1, 100)))))

  run first_checkpoint_works = replayInit.then(checkpointBlockAt(0))
    .then(replayBlock(List(hubWork(0, List())), 2))
  run middle_checkpoint_works = replayInit.then(checkpointBlockAt(1))
    .then(replayBlock(List(hubWork(0, List())), 2))
  run last_checkpoint_works = replayInit.then(checkpointBlockAt(2))
    .then(replayBlock(List(hubWork(0, List())), 2))

  run rejected_then_accepted_works = replayInit.then(all {
    val wr = hubWork(0, List())
    budgetedInvocation(List(wr, wr), 1, List(), List(reportGas(wr) - 1, reportGas(wr)), -1)
  })

  run refine_error_budget_works = replayInit.then(all {
    val wr = hubWork(2, List())
    budgetedInvocation(List(wr, wr, { result: WorkErr, authTrace: 0 }), 1, List(),
      List(4999999, 5000000, 0), -1)
  })
  run assignment_checkpoint_works = replayInit
    .then(replayBlock(List(replayWork(svc, CoretimeParaId, 0,
      activeCodeCandidate(svc, CoretimeParaId), 0, 0, 0,
      List(AssignCore({ core: 0, queue: List({ authBytes: 10 }),
        jamSlot: 2, newAssigner: None })))), 1))
    .then(checkpointBlockAt(0))
    .then(replayBlock(List(hubWork(0, List())), 3))

  run saturating_budget_works = replayInit
    .then(boundary(1, List(TransferOut(transfer(99, 0, 1, UnlimitedGas)))))
    .then(boundary(2, List(TransferOut(transfer(99, 0, 1, UnlimitedGas)))))
}
