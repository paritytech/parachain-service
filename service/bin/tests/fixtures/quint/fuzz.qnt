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

  pure def activeCodeCandidate(s: ParachainServiceState, para: ParaId): ParachainCandidate =
    { validationCode: { hash: activeCodeHashOr(s.paraInfo(para), NoCodeHash), len: FixedCodeLen }, pov: 0 }

  pure def replayWork(
    s: ParachainServiceState, para: ParaId, curHead: HeadData,
    candidate: ParachainCandidate, mode: int, anchor: Timeslot, auth: int,
    msgs: List[UpwardMessage]
  ): WorkResult = {
    val pending = match s.paraInfo(para).pendingUpgrade {
      | None => activeCodeCandidate(s, para)
      | Some(pair) => { validationCode: pair._1.ref, pov: 0 }
    }
    val pkg: WorkPackage = {
      authorizedParas: List(para),
      items: List(if (mode == 6) candidate else if (mode == 8) pending
        else activeCodeCandidate(s, para))
    }
    val result = if (mode == 1) WorkErr else refine(
      s, pkg, false,
      if (mode == 2) PvfReportError(42) else if (mode == 4) PvfPanic else PvfOk,
      if (mode == 3) None else Some(headHash(
        if (mode == 7) StaleParentHead else curHead)),
      if (mode == 5) HeadUndeclared else HeadDeclared(curHead + 1), msgs, anchor)[0]
    { result: result, authTrace: auth }
  }

  // A small shared preimage pool makes repeats and cross-para references common.
  // Keep one length per hash, as required by the replay codex.
  pure def replayMessages(s: ParachainServiceState, para: ParaId): Set[UpwardMessage] = {
    val code = vchAsHash(activeCodeHashOr(s.paraInfo(para), NoCodeHash))
    Set(
      RequestCodeUpgrade({ hash: { vchBytes: 777 }, len: FixedCodeLen }),
      RequestCodeUpgrade({ hash: { vchBytes: 778 }, len: FixedCodeLen }),
      RequestCodeUpgrade({ hash: activeCodeHashOr(s.paraInfo(para), NoCodeHash), len: FixedCodeLen }),
      Solicit({ target: Parachain(para), hash: { hashBytes: 777 }, len: FixedCodeLen }),
      Solicit({ target: Parachain(para), hash: { hashBytes: 778 }, len: FixedCodeLen }),
      Forget({ target: Parachain(para), hash: { hashBytes: 777 }, len: FixedCodeLen }),
      Forget({ target: Parachain(para), hash: { hashBytes: 778 }, len: FixedCodeLen }),
      Solicit({ target: Parachain(para), hash: { hashBytes: 7 }, len: 1024 }),
      Solicit({ target: Parachain(para), hash: { hashBytes: 8 }, len: 1024 }),
      Forget({ target: Parachain(para), hash: { hashBytes: 7 }, len: 1024 }),
      Forget({ target: Parachain(para), hash: { hashBytes: 8 }, len: 1024 }),
      Solicit({ target: Parachain(para), hash: code, len: FixedCodeLen }),
      Forget({ target: Parachain(para), hash: code, len: FixedCodeLen }),
      Forget({ target: Parachain(CoretimeParaId), hash: { hashBytes: 7 }, len: 1024 }),
      Forget({ target: Parachain(AssetHubParaId), hash: { hashBytes: 7 }, len: 1024 })
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
      FixedCodeLen, packages, foreignServices)
    all {
      svc' = out.state,
      foreignServices' = out.foreign,
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
