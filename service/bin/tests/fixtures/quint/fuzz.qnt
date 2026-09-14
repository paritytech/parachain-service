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

  import lifecycle_inputs.* from "./lifecycle_inputs"
  import transfer_inputs.* from "./transfer_inputs"

  action replayInit = all { init, noTransfers }

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
    // No leading-zero keys: Quint's base-257 listHash aliases those keys.
    val kvKeys = Set(List(), List(1), List(2, 0, 255))
    val values = Set(List(), List(0), List(255), List(0, 255),
      range(0, 63).foldl(List(), (acc, _) => acc.append(255)),
      range(0, 64).foldl(List(), (acc, _) => acc.append(255)))
    val kv = kvKeys.fold(Set(), (msgs, key) => msgs
      .union(values.map(value => SetKV({ key: key, value: value })))
      .union(Set(
        RemoveKV({ paraId: para, key: key }),
        RemoveKV({ paraId: CoretimeParaId, key: key }),
        RemoveKV({ paraId: AssetHubParaId, key: key }))))
    val balances = s.parachains.keys().fold(Set(), (msgs, target) => {
      val used = s.paraInfo(target).usedStateBalance
      val totals = Set(0, used - 1, used, used + 1,
        s.paraInfo(target).totalStateBalance, used + FixedCodeLen * 2)
      msgs.union(totals.map(total =>
        ParachainSetStateBalance({ paraId: target, newTotal: total })))
    })
    kv.union(balances).union(lifecycleMessages).union(Set(
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
    ))
  }

  action block = {
    nondet gap = oneOf(Set(1, 2, MaxLookupAge))
    val slot = now + gap
    val floor = if (slot > MaxLookupAge) slot - MaxLookupAge else 0
    nondet numPkgs = oneOf(Set(0, 1, 2, 3))
    nondet para1 = oneOf(svc.parachains.keys())
    nondet para2 = oneOf(svc.parachains.keys())
    nondet para3 = oneOf(svc.parachains.keys())
    // Choose whole outcome classes independently for each WP, keeping valid
    // work frequent enough to advance heads and prune logs.
    nondet mode1 = oneOf(0.to(8))
    nondet mode2 = oneOf(0.to(8))
    nondet mode3 = oneOf(0.to(8))
    nondet candidate1 = oneOf(arbitraryCandidates(svc, para1))
    nondet candidate2 = oneOf(arbitraryCandidates(svc, para2))
    nondet candidate3 = oneOf(arbitraryCandidates(svc, para3))
    nondet auth1 = oneOf(Set(0, 7, 255, 256))
    nondet auth2 = oneOf(Set(0, 7, 255, 256))
    nondet auth3 = oneOf(Set(0, 7, 255, 256))
    nondet anchor1 = oneOf(Set(floor, (floor + now) / 2, now))
    nondet anchor2 = oneOf(Set(floor, (floor + now) / 2, now))
    nondet anchor3 = oneOf(Set(floor, (floor + now) / 2, now))
    nondet numMsgs1 = oneOf(Set(0, 1, 2, 3, 4))
    nondet numMsgs2 = oneOf(Set(0, 1, 2, 3, 4))
    nondet numMsgs3 = oneOf(Set(0, 1, 2, 3, 4))
    nondet msg11 = oneOf(replayMessages(svc, para1))
    nondet msg12 = oneOf(replayMessages(svc, para1))
    nondet msg13 = oneOf(replayMessages(svc, para1))
    nondet msg14 = oneOf(replayMessages(svc, para1))
    nondet msg21 = oneOf(replayMessages(svc, para2))
    nondet msg22 = oneOf(replayMessages(svc, para2))
    nondet msg23 = oneOf(replayMessages(svc, para2))
    nondet msg24 = oneOf(replayMessages(svc, para2))
    nondet msg31 = oneOf(replayMessages(svc, para3))
    nondet msg32 = oneOf(replayMessages(svc, para3))
    nondet msg33 = oneOf(replayMessages(svc, para3))
    nondet msg34 = oneOf(replayMessages(svc, para3))
    val msgs1 = List(msg11, msg12, msg13, msg14).slice(0, numMsgs1)
    val msgs2 = List(msg21, msg22, msg23, msg24).slice(0, numMsgs2)
    val msgs3 = List(msg31, msg32, msg33, msg34).slice(0, numMsgs3)
    nondet chain = oneOf(Set(false, true))
    nondet chain3 = oneOf(Set(false, true))
    val head1 = svc.paraInfo(para1).headData
    // Same-para WPs either compete for the pre-block head or chain off the
    // latest preceding candidate's proposed head. All refine against pre-block
    // state, including when earlier work fails and the chained parent never lands.
    val head2 = if (para2 == para1 and chain) head1 + 1
      else svc.paraInfo(para2).headData
    val head3 = if (chain3 and para3 == para2) head2 + 1
      else if (chain3 and para3 == para1) head1 + 1
      else svc.paraInfo(para3).headData
    val wp1 = replayWork(svc, para1, head1, candidate1, mode1, anchor1, auth1, msgs1)
    val wp2 = replayWork(svc, para2, head2, candidate2, mode2, anchor2, auth2, msgs2)
    val wp3 = replayWork(svc, para3, head3, candidate3, mode3, anchor3, auth3, msgs3)
    val packages = List(wp1, wp2, wp3).slice(0, numPkgs)
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

  action incoming = {
    nondet sender = oneOf(Set(MkServiceId(0), MkServiceId(3), MkServiceId(7)))
    nondet amount = oneOf(Set(0, IncomingTransferEntryFootprint - 1,
      IncomingTransferEntryFootprint, IncomingTransferEntryFootprint + 1, 10000))
    nondet memo = oneOf(Set(0, 1, 255))
    nondet count = oneOf(Set(1, 2, 3))
    val transfers = range(0, count).foldl(List(), (acc, i) => acc.append({
      from: sender, amount: amount, toSupervisorBalance: false, memo: memo + i
    }))
    recordIncoming(transfers)
  }

  action replayStep = any {
    all { block, noTransfers }, all { provisionPreimage, noTransfers }, incoming,
    all { lifecycleBlock, noTransfers }
  }
}
