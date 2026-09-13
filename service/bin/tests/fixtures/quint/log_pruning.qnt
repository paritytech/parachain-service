// Log pruning sequences evaluated by the pinned Quint model.
module log_pruning {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"

  import tests.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests"

  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"

  action applyWork(result: WorkExecResult): bool = {
    val out = accumulateBlock(
      svc, now + 1, jamStagingSet, solicitedSet, FixedCodeLen,
      List({ result: result, authTrace: 0 }))
    all {
      svc' = out.state,
      now' = now + 1,
      jamStagingSet' = out.staging,
      solicitedSet' = out.solicited,
      prevSvc' = svc,
      lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns,
      lastHeadRoot' = out.headRoot,
      logPrunedBelow' =
        out.logPruned.foldl(logPrunedBelow, (m, pair) => m.put(pair._1, pair._2))
    }
  }

  action candidateAt(anchor: Timeslot): bool = {
    val pi = svc.paraInfo(CoretimeParaId)
    val pkg: WorkPackage = {
      authorizedParas: List(CoretimeParaId),
      items: List({ validationCode: { hash: { vchBytes: 1 }, len: FixedCodeLen }, pov: 0 })
    }
    applyWork(refine(svc, pkg, false, PvfOk,
      Some(headHash(pi.headData)), Some(pi.headData + 1), List(), anchor)[0])
  }

  run refine_log_boundary_works =
    init
      .then(applyWork(WorkOk(Err({ paraId: CoretimeParaId, error: MissingHeadDeclaration }))))
      .then(applyWork(WorkOk(Err({ paraId: CoretimeParaId, error: Opaque(42) }))))
      .then(expectThat(svc.parachainLog.get(CoretimeParaId).length() == 2))
      .then(candidateAt(2))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).headData == 1,
        svc.parachainLog.get(CoretimeParaId) == List(
          (2, RefineLogEntry({ error: Opaque(42), authTrace: 0 })))
      }))

  // Forgetting provided preimages succeeds by unrequesting them and recording
  // when the second forget may expunge them. These are not rejection events.
  run accumulate_log_boundary_works = {
    val first: Hash = { hashBytes: 701 }
    val second: Hash = { hashBytes: 702 }
    val len = 32
    init
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(
        Solicit({ hash: first, len: len }), Solicit({ hash: second, len: len }))))
      // Clear per-block work results before the external provision actions.
      .then(applyHappyPaths(List()))
      .then(provide(first, len))
      .then(provide(second, len))
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(
        Forget({ paraId: CoretimeParaId, hash: first, len: len }))))
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(
        Forget({ paraId: CoretimeParaId, hash: second, len: len }))))
      .then(expectThat(all {
        svc.parachainLog.get(CoretimeParaId) == List(
          (3, AccumulateLogEntry(List(ForgetAgainAt({ hash: first, len: len, due: 3 + ExpungePeriod })))),
          (4, AccumulateLogEntry(List(ForgetAgainAt({ hash: second, len: len, due: 4 + ExpungePeriod }))))),
        svc.preimageStatus.get((first, len)) == Unrequested(3),
        svc.preimageStatus.get((second, len)) == Unrequested(4)
      }))
      .then(candidateAt(4))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).headData == 4,
        svc.parachainLog.get(CoretimeParaId) == List(
          (4, AccumulateLogEntry(List(ForgetAgainAt({ hash: second, len: len, due: 4 + ExpungePeriod }))))),
        svc.preimageRegistry == prevSvc.preimageRegistry,
        svc.preimageStatus == prevSvc.preimageStatus,
        svc.paraInfo(CoretimeParaId).usedStateBalance == prevSvc.paraInfo(CoretimeParaId).usedStateBalance
      }))
  }
}
