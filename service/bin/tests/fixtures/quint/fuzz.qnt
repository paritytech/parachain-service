// Input generator for the supported one-WP replay profile. No expected states
// or transition sequences are prescribed here; semantics come from the pin.
module fuzz {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  action block = {
    nondet gap = oneOf(Set(1, 2, MaxLookupAge))
    val slot = now + gap
    val floor = if (slot > MaxLookupAge) slot - MaxLookupAge else 0
    nondet anchor = oneOf(Set(floor, (floor + now) / 2, now))
    nondet para = oneOf(svc.parachains.keys())
    // Choose a whole outcome class, so valid work is frequent enough to
    // advance heads and prune logs instead of mostly failing early checks.
    nondet mode = oneOf(0.to(8))
    nondet candidate = oneOf(arbitraryCandidates(svc, para))
    nondet auth = oneOf(Set(0, 7, 255, 256))
    val pi = svc.paraInfo(para)
    val pkg: WorkPackage = {
      authorizedParas: List(para),
      items: List(
        if (mode == 6) candidate else activeCodeCandidate(svc, para))
    }
    val result = if (mode == 1) WorkErr else refine(
      svc, pkg, false,
      if (mode == 2) PvfReportError(42) else if (mode == 4) PvfPanic else PvfOk,
      if (mode == 3) None else Some(headHash(
        if (mode == 7) StaleParentHead else pi.headData)),
      if (mode == 5) None else Some(pi.headData + 1), List(), anchor)[0]
    val out = accumulateBlock(svc, slot, jamStagingSet, solicitedSet,
      FixedCodeLen, if (mode == 0) List() else List({ result: result, authTrace: auth }))
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
