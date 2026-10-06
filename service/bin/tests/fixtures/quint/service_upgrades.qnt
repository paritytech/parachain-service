// Service self-upgrade scenarios; all decisions come from the pinned model.
module service_upgrades_replay {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  var replayDesignate: List[ValidatorKey]
  action check(ok: bool): bool = all { expectThat(ok), replayDesignate' = replayDesignate, replayTransfers' = replayTransfers, keepUpgradeGas }
  import outgoing_inputs.* from "./outgoing_inputs"
  import service_upgrade_inputs.* from "./service_upgrade_inputs"
  action start = all { outgoingInit, replayDesignate' = List(), replayMinAccGas' = 100 }
  def work(p: ParaId, parent: int, msgs: List[UpwardMessage]): WorkResult = {
    val h = if (p == AssetHubParaId) 2 else 1
    val pkg: WorkPackage = { authorizedParas: List(p),
      items: List({ validationCode: { hash: { vchBytes: h }, len: FixedCodeLen } }) }
    val calls = List(SetParentHeadHash(headHash(parent)), SetHead(parent + 1)).concat(sends(msgs))
    { result: refine(svc, pkg, false, calls, now)[0], authTrace: 0 }
  }
  action block(packages: List[WorkResult]): bool = {
    // With an empty incoming JAM set, out.staging is exactly the last
    // successful designation (or empty for no effect), even for identical sets.
    val out = accumulateBlock(svc, now + 1, List(), solicitedSet,
      FixedCodeLen, packages, foreignServices)
    val effects = messageEffects(packages, now + 1)
    all {
      svc' = out.state, now' = now + 1, prevSvc' = svc,
      replayDesignate' = out.staging,
      replayTransfers' = effects.transfers,
      replayMinAccGas' = effects.minAccGas.optionGetOr(replayMinAccGas),
      jamStagingSet' = if (out.staging == List()) jamStagingSet else out.staging,
      solicitedSet' = out.solicited, lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns, lastHeadRoot' = out.headRoot,
      foreignServices' = out.foreign,
      logPrunedBelow' = out.logPruned.foldl(logPrunedBelow, (m, pair) => m.put(pair._1, pair._2))
    }
  }
  action hub(m: UpwardMessage): bool = block(List(work(AssetHubParaId,
    svc.paraInfo(AssetHubParaId).headData, List(m))))

  pure val a: Hash = { hashBytes: 9001 }
  pure val b: Hash = { hashBytes: 9002 }
  pure def solicitCode(p: ParaId, hash: Hash): UpwardMessage =
    Solicit({ target: Parachain(p), hash: hash, len: ServiceCodeLen })
  pure def forgetCode(hash: Hash): UpwardMessage =
    Forget({ target: Parachain(AssetHubParaId), hash: hash, len: ServiceCodeLen })
  action core(msgs: List[UpwardMessage]): bool = block(List(work(CoretimeParaId,
    svc.paraInfo(CoretimeParaId).headData, msgs)))
  action fund = core(List(
    ParachainSetStateBalance({ paraId: AssetHubParaId, newTotal: InitialAssetHubBalance + 3 * ServiceCodeLen }),
    ParachainSetStateBalance({ paraId: CoretimeParaId, newTotal: InitialParaBalance + 3 * ServiceCodeLen })))
  action supplied(hash: Hash): bool = all {
    provide(hash, ServiceCodeLen), noOutgoing, replayDesignate' = List(), keepUpgradeGas
  }
  action ready(hash: Hash): bool = hub(solicitCode(AssetHubParaId, hash))

  run activation_works = start.then(fund).then(ready(a)).then(supplied(a))
    .then(hub(upgrade(a, 123456, 789)))
    .then(check(svc.serviceCodeHash == a and svc.serviceCodeLen == ServiceCodeLen and replayMinAccGas == 123456))
    .then(block(List())).then(hub(SetKV({ key: List(1), value: List(2) })))
  run missing_and_wrong_length_works = start.then(fund)
    .then(hub(upgrade(a, 11, 22)))
    .then(ready(a)).then(hub(upgrade(a, 33, 44))).then(supplied(a))
    .then(hub(UpgradeService({ codeHash: a, len: ServiceCodeLen + 1, minAccGas: 55, minMemoGas: 66 })))
    .then(check(svc.serviceCodeHash == { hashBytes: 0 } and replayMinAccGas == 100))
    .then(hub(upgrade(a, 0, 0))).then(block(List()))
  run unreferenced_works = start.then(fund)
    .then(core(List(solicitCode(CoretimeParaId, a)))).then(supplied(a))
    .then(hub(upgrade(a, 11, 22)))
    .then(check(svc.serviceCodeHash == { hashBytes: 0 }))
    .then(ready(a)).then(hub(upgrade(a, 33, 44))).then(block(List()))
  run forgotten_and_rerequested_works = start.then(fund).then(ready(a)).then(supplied(a))
    .then(hub(forgetCode(a))).then(hub(upgrade(a, 11, 22)))
    .then(check(svc.serviceCodeHash == { hashBytes: 0 }))
    .then(ready(a)).then(hub(upgrade(a, 33, 44))).then(block(List()))
  run running_code_protected_works = start.then(fund).then(ready(a)).then(supplied(a))
    .then(hub(upgrade(a, 11, 22))).then(hub(forgetCode(a)))
    .then(check(svc.preimageStatus.get((a, ServiceCodeLen)) == Provided))
    .then(ready(b)).then(supplied(b)).then(hub(upgrade(b, 33, 44)))
    .then(hub(forgetCode(a))).then(ready(a)).then(hub(upgrade(a, 55, 66)))
    .then(block(List()))
  run same_code_new_gas_works = start.then(fund).then(ready(a)).then(supplied(a))
    .then(hub(upgrade(a, 11, 22))).then(hub(upgrade(a, 33, 44)))
    .then(check(svc.serviceCodeHash == a and replayMinAccGas == 33 and svc.jamAccount.minMemoGas == 44))
    .then(block(List()))
  run ordered_upgrades_works = start.then(fund).then(ready(a)).then(supplied(a))
    .then(ready(b)).then(supplied(b))
    .then(block(List(work(AssetHubParaId, svc.paraInfo(AssetHubParaId).headData,
      List(upgrade(a, 11, 22), upgrade(b, 33, 44))),
      work(AssetHubParaId, svc.paraInfo(AssetHubParaId).headData + 1,
        List(upgrade(a, 55, 66), UpgradeService({ codeHash: b, len: ServiceCodeLen + 1,
          minAccGas: 77, minMemoGas: 88 }))))))
    .then(check(svc.serviceCodeHash == a and replayMinAccGas == 55 and svc.jamAccount.minMemoGas == 66))
    .then(block(List()))
  run rejected_work_works = start.then(fund).then(ready(a)).then(supplied(a))
    .then(block(List(work(CoretimeParaId, svc.paraInfo(CoretimeParaId).headData, List(upgrade(a, 11, 22))),
      work(AssetHubParaId, 999, List(upgrade(a, 33, 44))), { result: WorkErr, authTrace: 0 })))
    .then(check(svc.serviceCodeHash == { hashBytes: 0 } and replayMinAccGas == 100))
    .then(hub(upgrade(a, 55, 66))).then(block(List()))
  run same_package_guard_works = start.then(fund).then(ready(a)).then(supplied(a))
    .then(ready(b)).then(supplied(b))
    .then(block(List(work(AssetHubParaId, svc.paraInfo(AssetHubParaId).headData,
      List(upgrade(a, 11, 22), forgetCode(a), upgrade(b, 33, 44), forgetCode(a), forgetCode(b))))))
    .then(check(svc.serviceCodeHash == b and svc.preimageStatus.get((b, ServiceCodeLen)) == Provided))
    .then(block(List()))
}
