// Lifecycle input vocabulary; all outcomes are evaluated by the pinned model.
module lifecycle_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  pure val lifecycleParas = Set(MkParaId(3), MkParaId(4))
  pure def fund(p: ParaId, amount: int): UpwardMessage =
    ParachainSetStateBalance({ paraId: p, newTotal: amount })
  pure def setHead(p: ParaId, h: int): UpwardMessage =
    ParachainSetHead({ paraId: p, newHead: h })
  pure def code(p: ParaId, h: int): UpwardMessage =
    ParachainSetValidationCode({ paraId: p,
      newValidationCodeHash: { vchBytes: h }, newValidationCodeLen: FixedCodeLen })
  pure val lifecycleMessages = lifecycleParas.fold(Set(), (acc, p) => acc.union(Set(
    fund(p, baselineFor(p) - 1), fund(p, baselineFor(p)), fund(p, InitialParaBalance),
    setHead(p, 0), setHead(p, 42), code(p, 777), code(p, 778), ParachainCleanUp(p))))

  // Unlike the upstream happy-path helper, this also accepts an absent para.
  // Its candidate still passes through model Refine and Accumulate.
  action lifecycleWork(p: ParaId, h: int, msgs: List[UpwardMessage], gap: int): bool = {
    val parent = if (svc.isLive(p)) svc.paraInfo(p).headData else 0
    val pkg: WorkPackage = { authorizedParas: List(p),
      items: List({ validationCode: { hash: { vchBytes: h }, len: FixedCodeLen }, pov: 0 }) }
    val wr = { result: refine(svc, pkg, false, PvfOk, Some(headHash(parent)),
      HeadDeclared(parent + 1), msgs, now)[0], authTrace: 0 }
    val out = accumulateBlock(svc, now + gap, jamStagingSet, solicitedSet,
      FixedCodeLen, List(wr), foreignServices)
    all {
      svc' = out.state, now' = now + gap, prevSvc' = svc,
      jamStagingSet' = out.staging, solicitedSet' = out.solicited,
      lastStepWorkResults' = out.results, lastStepAssigns' = out.assigns,
      lastHeadRoot' = out.headRoot, foreignServices' = out.foreign,
      logPrunedBelow' = out.logPruned.foldl(logPrunedBelow, (m, pair) => m.put(pair._1, pair._2))
    }
  }

  action lifecycleBlock = {
    nondet p = oneOf(lifecycleParas)
    nondet caller = oneOf(Set(CoretimeParaId, AssetHubParaId))
    nondet mode = oneOf(Set(0, 1, 2, 3))
    nondet msg = oneOf(lifecycleMessages)
    nondet gap = oneOf(Set(1, ExpungePeriod, ExpungePeriod + 1))
    val h = activeCodeHashOr(svc.paraInfo(caller), NoCodeHash).vchBytes
    if (mode == 0) lifecycleWork(caller, h,
      List(fund(p, InitialParaBalance), setHead(p, 0), code(p, 777)), gap)
    else if (mode == 1) lifecycleWork(caller, h, List(ParachainCleanUp(p)), gap)
    else if (mode == 2) lifecycleWork(caller, h, List(msg), gap)
    else lifecycleWork(p,
      if (svc.isLive(p)) activeCodeHashOr(svc.paraInfo(p), NoCodeHash).vchBytes else 777,
      List(SetKV({ key: List(1), value: List(1) })), gap)
  }
}
