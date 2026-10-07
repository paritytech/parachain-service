// Replay effects are evaluated by the pinned model, including same-set designation.
module designation_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import head_commitment.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/head_commitment"
  var replayCanDesignate: bool
  var replayDesignate: List[ValidatorKey]
  action check(ok: bool): bool = all { expectThat(ok), replayDesignate' = replayDesignate, replayCanDesignate' = replayCanDesignate }
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
    replayDesignate' = List(), replayCanDesignate' = true
  }
  def work(p: ParaId, parent: int, msgs: List[UpwardMessage]): WorkResult = {
    val h = if (p == AssetHubParaId) 2 else 1
    val pkg: WorkPackage = { authorizedParas: List(p),
      items: List({ validationCode: { hash: { vchBytes: h }, len: FixedCodeLen } }) }
    val calls = List(SetParentHeadHash(headHash(parent)), SetHead(parent + 1)).concat(sends(msgs))
    { result: refine(svc, pkg, false, calls, now)[0], authTrace: 0 }
  }
  // The pin models valid key counts but assumes designation privilege.
  // Preserve its staging/length semantics, replacing only a successful host
  // designation with the corresponding rejection when the input denies it.
  pure def applyPrivilege(acc: AccumulateOutcome, caller: ParaId, slot: int,
    anchor: int, message: UpwardMessage, allowed: bool): AccumulateOutcome = {
    val out = applyUpwardMessage(acc, caller, FixedCodeLen, slot, anchor, message)
    match message {
      | SetValidatorKeys(k) => {
        val len = acc.state.stagedValidatorKeys.length() + k.keys.length()
        if (not(allowed) and k.isLast and k.keys != List() and ValCount.contains(len))
          { ...out, jamDesignate: acc.jamDesignate,
            logs: acc.logs.append(DesignateRejected({ len: len })) }
        else out
      }
      | _ => out
    }
  }
  action designationBlock(allowed: bool, messages: List[UpwardMessage]): bool = {
    val slot = now + 1
    val wr = work(AssetHubParaId, svc.paraInfo(AssetHubParaId).headData, messages)
    val plain = accumulateOnePackage(svc, wr, solicitedSet, List(), FixedCodeLen, slot, foreignServices)
    val out = match wr.result {
      | WorkOk(digest) => match digest {
        | Ok(ok) => {
          val pre = accumulateOkPrefix(svc, ok, FixedCodeLen, slot)
          val base: AccumulateOutcome = { state: pre.state, foreign: foreignServices,
            jamDesignate: List(), preimageOps: List(), jamAssigns: List(), logs: pre.logs }
          val applied = messages.foldl(base, (a, m) => applyPrivilege(a, ok.paraId, slot, ok.lookupAnchor, m, allowed))
          { ...applied, state: applied.state.appendAccumulateLogs(ok.paraId, slot, applied.logs) }
        }
        | _ => { state: plain.state, foreign: foreignServices, jamDesignate: List(), preimageOps: List(), jamAssigns: List(), logs: List() }
      }
      | _ => { state: plain.state, foreign: foreignServices, jamDesignate: List(), preimageOps: List(), jamAssigns: List(), logs: List() }
    }
    all {
      svc' = out.state, prevSvc' = svc, now' = slot,
      replayCanDesignate' = allowed, replayDesignate' = out.jamDesignate,
      jamStagingSet' = if (out.jamDesignate == List()) jamStagingSet else out.jamDesignate,
      solicitedSet' = solicitedSet, foreignServices' = foreignServices,
      lastStepWorkResults' = List(wr), lastStepAssigns' = List(),
      lastHeadRoot' = headCommitment(svc, out.state),
      logPrunedBelow' = match plain.logPruned {
        | None => logPrunedBelow | Some(p) => logPrunedBelow.put(p._1, p._2)
      }
    }
  }
  action designationStep = {
    nondet allowed = oneOf(Set(false, true))
    nondet count = oneOf(Set(0, 1, 3, 5, 6, 30))
    nondet last = oneOf(Set(false, true))
    nondet repeat = oneOf(Set(false, true))
    val message = msg(count, now, last)
    designationBlock(allowed, if (repeat) List(message, msg(6, now + 100, true)) else List(message))
  }
}
