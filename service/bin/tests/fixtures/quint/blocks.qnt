// Block sequences evaluated by the pinned Quint model.
module blocks {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"

  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"

  import accumulate_test.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/accumulate_test"

  import head_commitment_test.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/head_commitment_test"

  action emptyBlock = applyHappyPaths(List())
  action candidate = applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List())
  action applyWork(result: WorkExecResult, authTrace: int): bool = {
    val out = accumulateBlock(
      svc, now + 1, jamStagingSet, solicitedSet, FixedCodeLen,
      List({ result: result, authTrace: authTrace }), foreignServices)
    all {
      svc' = out.state,
      foreignServices' = out.foreign,
      now' = now + 1,
      jamStagingSet' = out.staging,
      solicitedSet' = out.solicited,
      prevSvc' = svc,
      lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns,
      lastHeadRoot' = out.headRoot,
      logPrunedBelow' = logPrunedBelow
    }
  }

  action logError(error: RefineLog, authTrace: int): bool =
    applyWork(WorkOk(Err({ paraId: CoretimeParaId, error: error })), authTrace)

  run stale_parent_candidate_rejected_works =
    staleParentCandidateRejectedTest

  run minimal_replay_works =
    init.then(candidate).then(expectThat(svc.paraInfo(CoretimeParaId).headData == 1))

  run two_work_packages_works =
    multipleWorkPackagesAccumulateTest

  // Both paras share a preimage. The second WP then releases Asset Hub's
  // reference, which must see the first WP's solicitation in the same block.
  run ump_ordering_works = {
    val h = { hashBytes: 7 }
    pure def solicit(para: ParaId): UpwardMessage = Solicit({ target: Parachain(para), hash: h, len: 1024 })
    val forgetHub = Forget({ target: Parachain(AssetHubParaId), hash: h, len: 1024 })
    val forgetCore = Forget({ target: Parachain(CoretimeParaId), hash: h, len: 1024 })
    init
      .then(applyHappyPaths(List(
        (AssetHubParaId, { vchBytes: 2 }, List(solicit(AssetHubParaId), solicit(AssetHubParaId))),
        (CoretimeParaId, { vchBytes: 1 }, List(solicit(CoretimeParaId), forgetHub))
      )))
      .then(expectThat(svc.preimageRegistry.get((h, 1024)).referencers == Set(CoretimeParaId)))
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(forgetCore)))
      .then(expectThat(not(svc.preimageRegistry.keys().contains((h, 1024)))))
      .then(applyHappyPath(AssetHubParaId, { vchBytes: 2 }, List(forgetHub, solicit(AssetHubParaId))))
      .then(expectThat(svc.preimageRegistry.get((h, 1024)).referencers == Set(AssetHubParaId)))
  }

  run rejected_state_balance_works =
    init.then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(
      ParachainSetStateBalance({ paraId: MkParaId(3000), newTotal: 1 }))))
      .then(expectThat(not(svc.isLive(MkParaId(3000)))))

  run work_error_between_candidates_works =
    init.then(candidate).then(expectThat(svc.paraInfo(CoretimeParaId).headData == 1))
      .then(applyWork(WorkErr, 7))
      .then(expectThat(svc == prevSvc and svc.paraInfo(CoretimeParaId).headData == 1))
      .then(candidate).then(expectThat(svc.paraInfo(CoretimeParaId).headData == 2))

  run empty_blocks_between_errors_works =
    init.then(emptyBlock).then(expectThat(svc == prevSvc))
      .then(logError(MissingHeadDeclaration, 7))
      .then(emptyBlock).then(expectThat(svc == prevSvc))
      .then(emptyBlock).then(expectThat(svc == prevSvc))
      .then(logError(Opaque(42), 255))
      .then(emptyBlock).then(expectThat(svc == prevSvc))

  run empty_blocks_between_candidates_works =
    init.then(candidate).then(expectThat(svc.paraInfo(CoretimeParaId).headData == 1))
      .then(emptyBlock).then(expectThat(svc == prevSvc))
      .then(emptyBlock).then(expectThat(svc == prevSvc))
      .then(candidate).then(expectThat(svc.paraInfo(CoretimeParaId).headData == 2))
      .then(emptyBlock).then(expectThat(svc == prevSvc))

  run empty_blocks_between_error_and_candidate_works =
    init.then(logError(MissingHeadDeclaration, 7))
      .then(emptyBlock).then(expectThat(svc == prevSvc))
      .then(candidate).then(expectThat(svc.paraInfo(CoretimeParaId).headData == 1))
      .then(emptyBlock).then(expectThat(svc == prevSvc))
      .then(logError(Opaque(42), 7))
}
