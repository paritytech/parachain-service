// Input generator for the supported replay profile. No expected states
// or transition sequences are prescribed here; semantics come from the pin.
module fuzz {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  pure def replayWork(
    s: ParachainServiceState, para: ParaId, curHead: HeadData,
    candidate: ParachainCandidate, mode: int, anchor: Timeslot, auth: int,
    msgs: List[UpwardMessage]
  ): WorkResult = {
    val pkg: WorkPackage = {
      authorizedParas: List(para),
      items: List(if (mode == 6) candidate else activeCodeCandidate(s, para))
    }
    val result = if (mode == 1) WorkErr else refine(
      s, pkg, false,
      if (mode == 2) PvfReportError(42) else if (mode == 4) PvfPanic else PvfOk,
      if (mode == 3) None else Some(headHash(
        if (mode == 7) StaleParentHead else curHead)),
      if (mode == 5) None else Some(curHead + 1), msgs, anchor)[0]
    { result: result, authTrace: auth }
  }

  // A small shared preimage pool makes repeats and cross-para references common.
  // Keep one length per hash, as required by the replay codex.
  pure def replayMessages(s: ParachainServiceState, para: ParaId): Set[UpwardMessage] = {
    val code = vchAsHash(activeCodeHashOr(s.paraInfo(para), NoCodeHash))
    Set(
      Solicit({ hash: { hashBytes: 7 }, len: 1024 }),
      Solicit({ hash: { hashBytes: 8 }, len: 1024 }),
      Forget({ paraId: para, hash: { hashBytes: 7 }, len: 1024 }),
      Forget({ paraId: para, hash: { hashBytes: 8 }, len: 1024 }),
      Solicit({ hash: code, len: FixedCodeLen }),
      Forget({ paraId: para, hash: code, len: FixedCodeLen }),
      Forget({ paraId: CoretimeParaId, hash: { hashBytes: 7 }, len: 1024 }),
      Forget({ paraId: AssetHubParaId, hash: { hashBytes: 7 }, len: 1024 })
    )
  }

  action block = {
    nondet gap = oneOf(Set(1, 2, MaxLookupAge))
    val slot = now + gap
    val floor = if (slot > MaxLookupAge) slot - MaxLookupAge else 0
    nondet numPkgs = oneOf(Set(0, 1, 2))
    nondet para1 = oneOf(svc.parachains.keys())
    nondet para2 = oneOf(svc.parachains.keys())
    // Choose whole outcome classes independently for each WP, keeping valid
    // work frequent enough to advance heads and prune logs.
    nondet mode1 = oneOf(0.to(8))
    nondet mode2 = oneOf(0.to(8))
    nondet candidate1 = oneOf(arbitraryCandidates(svc, para1))
    nondet candidate2 = oneOf(arbitraryCandidates(svc, para2))
    nondet auth1 = oneOf(Set(0, 7, 255, 256))
    nondet auth2 = oneOf(Set(0, 7, 255, 256))
    nondet anchor1 = oneOf(Set(floor, (floor + now) / 2, now))
    nondet anchor2 = oneOf(Set(floor, (floor + now) / 2, now))
    nondet numMsgs1 = oneOf(Set(0, 1, 2))
    nondet numMsgs2 = oneOf(Set(0, 1, 2))
    nondet msg11 = oneOf(replayMessages(svc, para1))
    nondet msg12 = oneOf(replayMessages(svc, para1))
    nondet msg21 = oneOf(replayMessages(svc, para2))
    nondet msg22 = oneOf(replayMessages(svc, para2))
    val msgs1 = if (numMsgs1 == 0) List() else if (numMsgs1 == 1) List(msg11)
      else List(msg11, msg12)
    val msgs2 = if (numMsgs2 == 0) List() else if (numMsgs2 == 1) List(msg21)
      else List(msg21, msg22)
    nondet chain = oneOf(Set(false, true))
    val head1 = svc.paraInfo(para1).headData
    // Same-para WPs either compete for the pre-block head or chain off the
    // first candidate's proposed head. Both refine against pre-block state,
    // including when the first WP fails and the chained parent never lands.
    val head2 = if (para2 == para1 and chain) head1 + 1
      else svc.paraInfo(para2).headData
    val wp1 = replayWork(svc, para1, head1, candidate1, mode1, anchor1, auth1, msgs1)
    val wp2 = replayWork(svc, para2, head2, candidate2, mode2, anchor2, auth2, msgs2)
    val packages = if (numPkgs == 0) List()
      else if (numPkgs == 1) List(wp1) else List(wp1, wp2)
    val out = accumulateBlock(svc, slot, jamStagingSet, solicitedSet,
      FixedCodeLen, packages)
    all {
      svc' = out.state,
      now' = slot,
      jamStagingSet' = out.staging,
      solicitedSet' = out.solicited,
      prevSvc' = svc,
      lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns,
      lastHeadRoot' = out.headRoot,
      logPrunedBelow' = out.logPruned.foldl(logPrunedBelow,
        (m, pair) => m.put(pair._1, pair._2))
    }
  }

  action replayStep = any { block, provisionPreimage }
}
