module service_preimages {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import fuzz.* from "./fuzz"
  import service_inputs.* from "./service_inputs"
  import outgoing_inputs.* from "./outgoing_inputs"
  import invocation_inputs.* from "./invocation_inputs"
  import gas_inputs.* from "./gas_inputs"

  pure def pair(id: ServiceId): List[UpwardMessage] = List(serviceSolicit(id), serviceForget(id))
  def work(msgs: List[UpwardMessage], parent: int): WorkResult =
    replayWork(svc, AssetHubParaId, parent, activeCodeCandidate(svc, AssetHubParaId), 0, now, 0, msgs)
  action normal(msgs: List[UpwardMessage]): bool =
    replayBlock(List(work(msgs, svc.paraInfo(AssetHubParaId).headData)), now + 1)
  action gasBlock(mode: int): bool = {
    val wr = work(pair(MkServiceId(7)), svc.paraInfo(AssetHubParaId).headData)
    budgetedInvocation(List(wr), now + 1, List(), List(budget(wr, mode)), -1)
  }
  action interrupted(stop: int): bool = {
    val parent = svc.paraInfo(AssetHubParaId).headData
    val packages = range(0, 3).foldl(List(), (list, i) => list.append(work(
      pair(MkServiceId(42)).append(TransferOut(transfer(0, 1, i,
        if (i == stop) ExhaustPoolGas else 100))), parent + i)))
    budgetedInvocation(packages, now + 1, List(),
      List(UnlimitedGas, UnlimitedGas, UnlimitedGas), stop)
  }
  run refusals_works = replayInit
    .then(normal(pair(MkServiceId(99))))
    .then(normal(pair(MkServiceId(7))))
    .then(normal(pair(MkServiceId(0))))
    .then(normal(pair(ParachainServiceId)))
  run created_request_works = replayInit
    .then(normal(List(CreateService(createArgs(Some(MkServiceId(42)), 77)))
      .concat(pair(MkServiceId(42)))))
    .then(normal(pair(MkServiceId(42))))
  run unauthorized_works = replayInit.then(replayBlock(List(replayWork(svc, CoretimeParaId,
    svc.paraInfo(CoretimeParaId).headData, activeCodeCandidate(svc, CoretimeParaId), 0, now, 0,
    pair(MkServiceId(7)))), now + 1))
  run stale_work_works = replayInit.then(replayBlock(List(replayWork(svc, AssetHubParaId,
    svc.paraInfo(AssetHubParaId).headData, activeCodeCandidate(svc, AssetHubParaId), 7, now, 0,
    pair(MkServiceId(7)))), now + 1))
  run gas_rejection_works = replayInit.then(gasBlock(1)).then(gasBlock(2))
  run checkpoint_first_works = replayInit.then(interrupted(0)).then(normal(pair(MkServiceId(42))))
  run checkpoint_middle_works = replayInit.then(interrupted(1)).then(normal(pair(MkServiceId(42))))
  run checkpoint_last_works = replayInit.then(interrupted(2)).then(normal(pair(MkServiceId(42))))
  run host_budget_works = replayInit
    .then(hostBlockAt(1000, 0, 4, false, serviceSolicit(MkServiceId(7)), serviceForget(MkServiceId(99))))
    .then(hostBlockAt(2000, 0, 4, true, serviceSolicit(ParachainServiceId), serviceForget(ParachainServiceId)))
    .then(normal(pair(MkServiceId(7))))
}
