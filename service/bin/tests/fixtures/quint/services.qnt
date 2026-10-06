module services {
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
  import transfer_inputs.* from "./transfer_inputs"
  import validator_key_inputs.* from "./validator_key_inputs"
  import service_upgrade_inputs.* from "./service_upgrade_inputs"

  pure def create(desired: Option[ServiceId]): UpwardMessage = CreateService(createArgs(desired, 77))
  pure def eject(id: int): UpwardMessage = EjectService({ service: MkServiceId(id) })
  def work(msgs: List[UpwardMessage], parent: int): WorkResult =
    replayWork(svc, AssetHubParaId, parent, activeCodeCandidate(svc, AssetHubParaId), 0, now, 0, msgs)
  action normal(msgs: List[UpwardMessage]): bool = replayBlock(
    List(work(msgs, svc.paraInfo(AssetHubParaId).headData)), now + 1)
  action gasBlock(mode: int): bool = {
    val wr = work(List(create(None), eject(65536)), svc.paraInfo(AssetHubParaId).headData)
    budgetedInvocation(List(wr), now + 1, List(), List(budget(wr, mode)), -1)
  }
  action interrupted(stop: int): bool = {
    val parent = svc.paraInfo(AssetHubParaId).headData
    val packages = range(0, 3).foldl(List(), (list, i) => list.append(work(
      List(create(None), TransferOut(transfer(0, 1, i, if (i == stop) ExhaustPoolGas else 100))), parent + i)))
    budgetedInvocation(packages, now + 1,
      List({ from: MkServiceId(7), amount: 1000, toSupervisorBalance: false, memo: 0 }),
      List(UnlimitedGas, UnlimitedGas, UnlimitedGas), stop)
  }
  run host_balance_works = replayInit
    .then(hostBlockAt(1000, 0, 4, false, create(None), create(None)))
    .then(hostBlockAt(2000, 0, 4, true, create(None), eject(65536)))
    .then(normal(List(create(None), eject(65536))))
  run code_and_gas_boundaries_works = replayInit
    .then(normal(List(
      CreateService({ ...createArgs(None, 0), codeHash: { hashBytes: 8802 }, len: 0,
        minItemGas: 0, minMemoGas: 0 }),
      CreateService({ ...createArgs(None, 18446744073709551615), codeHash: { hashBytes: 8803 }, len: 64,
        minItemGas: 18446744073709551615, minMemoGas: 18446744073709551615 }))))
  run allocation_works = replayInit
    .then(normal(List(create(None), create(None), create(Some(MkServiceId(70000))))))
    .then(normal(List(create(None), create(Some(MkServiceId(42))), eject(42))))
    .then(normal(List(eject(65536), TransferOut(transfer(42, 100, 88, 200)))))
  run protected_collision_works = replayInit
    .then(normal(List(create(Some(MkServiceId(42))), create(Some(MkServiceId(42))))))
    .then(normal(List(create(Some(ParachainServiceId)), create(Some(MkServiceId(0))))))
  run balance_selectors_works = replayInit
    .then(normal(List(CreateService({ ...createArgs(None, 1), sourceSupervisorBalance: true }),
      CreateService({ ...createArgs(None, 2), newSupervisorBalance: true }),
      CreateService({ ...createArgs(None, 3), sourceSupervisorBalance: true, newSupervisorBalance: true }))))
  run ejection_refusals_works = replayInit
    .then(normal(List(eject(1), eject(99), eject(0), eject(7))))
    .then(normal(List(create(Some(MkServiceId(42))), eject(42))))
    .then(normal(List(eject(42), eject(42))))
  run gas_rejection_works = replayInit.then(gasBlock(1)).then(gasBlock(2))
  run checkpoint_first_works = replayInit.then(interrupted(0)).then(normal(List(create(None))))
  run checkpoint_middle_works = replayInit.then(interrupted(1)).then(normal(List(create(None))))
  run checkpoint_last_works = replayInit.then(interrupted(2)).then(normal(List(create(None))))
  run unauthorized_works = replayInit.then(replayBlock(List(replayWork(svc, CoretimeParaId,
    svc.paraInfo(CoretimeParaId).headData, activeCodeCandidate(svc, CoretimeParaId), 0, now, 0,
    List(create(None), eject(7)))), now + 1))
}
