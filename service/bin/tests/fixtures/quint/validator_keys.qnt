// Replay effects are evaluated by the pinned model, including same-set designation.
module validator_keys_replay {
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
  action check(ok: bool): bool = all { expectThat(ok), replayDesignate' = replayDesignate }
  pure def keyList(n: int, start: int): List[ValidatorKey] =
    range(0, n).foldl(List(), (acc, i) => acc.append(start + i))
  pure def msg(n: int, start: int, last: bool): UpwardMessage =
    SetValidatorKeys({ keys: keyList(n, start), isLast: last })
  action start(n: int): bool = all {
    svc' = { ...initialState, stagedValidatorKeys: keyList(n, 0) },
    prevSvc' = { ...initialState, stagedValidatorKeys: keyList(n, 0) },
    now' = 0, jamStagingSet' = List(), solicitedSet' = Set(),
    lastStepWorkResults' = List(), lastStepAssigns' = List(),
    lastHeadRoot' = None, foreignServices' = Map(), logPrunedBelow' = Map(),
    replayDesignate' = List()
  }
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
    all {
      svc' = out.state, now' = now + 1, prevSvc' = svc,
      replayDesignate' = out.staging,
      jamStagingSet' = if (out.staging == List()) jamStagingSet else out.staging,
      solicitedSet' = out.solicited, lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns, lastHeadRoot' = out.headRoot,
      foreignServices' = out.foreign,
      logPrunedBelow' = out.logPruned.foldl(logPrunedBelow, (m, pair) => m.put(pair._1, pair._2))
    }
  }
  action hub(m: UpwardMessage): bool = block(List(work(AssetHubParaId,
    svc.paraInfo(AssetHubParaId).headData, List(m))))

  run chunks_works = start(0)
    .then(hub(msg(30, 0, false))).then(hub(msg(30, 30, true)))
    .then(check(jamStagingSet == keyList(60, 0) and svc.stagedValidatorKeys == List()))
    .then(block(List()))
  run same_set_works = start(0)
    .then(hub(msg(6, 0, true))).then(hub(msg(6, 0, true)))
    .then(check(replayDesignate == keyList(6, 0)))
  run abort_works = start(30)
    .then(hub(msg(0, 0, true)))
    .then(check(svc.stagedValidatorKeys == List() and jamStagingSet == List()))
    .then(hub(msg(0, 0, true))).then(hub(msg(0, 0, false)))
  run invalid_lengths_works = start(30)
    .then(hub(msg(1, 30, true))).then(hub(msg(5, 0, true)))
    .then(check(svc.stagedValidatorKeys == List() and jamStagingSet == List()))
  run full_set_works = start(1000)
    .then(hub(msg(23, 1000, true)))
    .then(check(jamStagingSet == keyList(1023, 0)))
  run overflow_works = start(1023)
    .then(hub(msg(1, 1023, false)))
    .then(check(svc.stagedValidatorKeys == keyList(1023, 0)))
    .then(hub(msg(3, 1023, true)))
    .then(check(svc.stagedValidatorKeys == List() and jamStagingSet == List()))
  run authorization_works = start(6)
    .then(block(List(work(CoretimeParaId, 0, List(msg(6, 10, true))))))
    .then(check(svc.stagedValidatorKeys == keyList(6, 0) and jamStagingSet == List()))
    .then(hub(msg(6, 10, true)))
  run stale_work_works = start(6)
    .then(block(List(work(AssetHubParaId, 99, List(msg(6, 10, true))))))
    .then(check(svc.stagedValidatorKeys == keyList(6, 0) and jamStagingSet == List()))
  run replacement_order_works = start(0)
    .then(block(List(work(AssetHubParaId, 0, List(msg(6, 0, true))),
      work(AssetHubParaId, 1, List(msg(6, 10, true))),
      work(AssetHubParaId, 2, List(msg(0, 0, true))))))
    .then(check(jamStagingSet == keyList(6, 10)))
  run rejected_final_after_success_works = start(0)
    .then(block(List(work(AssetHubParaId, 0, List(msg(6, 0, true))),
      work(AssetHubParaId, 1, List(msg(1, 10, true))))))
    .then(check(jamStagingSet == keyList(6, 0)))
}
