module host_invocation {
  import foreign_services.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/foreign_services"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  import head_commitment.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/head_commitment"
  import transfer_inputs.* from "./transfer_inputs"
  import validator_key_inputs.* from "./validator_key_inputs"
  import outgoing_inputs.* from "./outgoing_inputs"
  import service_upgrade_inputs.* from "./service_upgrade_inputs"
  import gas_inputs.* from "./gas_inputs"
  import host_metadata.* from "./host_metadata"
  import service_inputs.* from "./service_inputs"
  import host_sizes.* from "./host_sizes"
  type HostAcc = { acc: AccumulateOutcome, free: int, safe: bool,
    rejects: int, minGas: Option[int], creations: List[CreationEffect] }
  pure def moved(h: HostAcc, acc: AccumulateOutcome): HostAcc = {
    val delta = hostCost(acc.state) - hostCost(h.acc.state)
      + h.acc.state.jamAccount.balance - acc.state.jamAccount.balance
    { ...h, acc: acc, free: h.free - delta, safe: h.safe and delta <= h.free }
  }
  pure def limitedMessage(h: HostAcc, caller: ParaId, slot: int, anchor: int,
    message: UpwardMessage): HostAcc = {
    val applied = applyUpwardMessage(h.acc, caller, FixedCodeLen, slot, anchor, message)
    match message {
      | CreateService(c) => {
        val cost = BaseDeposit + 2 * ItemDeposit + PreimageRequestOverhead + c.len
        val out = if (cost > h.free)
          { ...h.acc, logs: h.acc.logs.append(ServiceCreation({ id: c.id, result: CannotAfford })) }
          else replayApply(h.acc, caller, FixedCodeLen, slot, anchor, message)
        val nextHost = moved(h, out)
        { ...nextHost, creations: out.foreign.keys().exclude(h.acc.foreign.keys())
          .fold(h.creations, (list, id) => list.append({ service: id, args: c })) }
      }
      | EjectService(_) => moved(h, applied)
      | SetKV(kv) => {
        val before = h.acc.state
        val pi = before.paraInfo(caller)
        val existing = before.keyValueStorage.keys().contains((caller, kv.key))
        val delta = if (existing) kv.value.length() - before.keyValueStorage.get((caller, kv.key)).length()
          else kvEntryFootprint(kv.key.length(), kv.value.length())
        val charged = { ...pi, usedStateBalance: pi.usedStateBalance + delta }
        val piGrowth = paraBytes(charged) - paraBytes(pi)
        val storageGrowth = if (existing) delta else storeCost(5 + kv.key.length(), kv.value.length())
        // The private reservation check precedes both host writes. The charged
        // ParaInfo write precedes KV, and must also fit at that intermediate point.
        val hostRejected = applied.logs == h.acc.logs and
          (piGrowth > h.free or piGrowth + storageGrowth > h.free)
        if (hostRejected) { ...h, rejects: h.rejects + 1, acc: { ...h.acc,
          logs: h.acc.logs.append(InsufficientStateBalance(FromSetKV({ keyHash: listHash(kv.key) }))) } }
        else moved(h, applied)
      }
      | UpgradeService(p) => {
        val nextHost = moved(h, applied)
        { ...nextHost, minGas: if (applied.logs == h.acc.logs) Some(p.minAccGas) else h.minGas }
      }
      | RemoveServiceStorage(_) => moved(h, replayApply(h.acc, caller, FixedCodeLen, slot, anchor, message))
      | SetServiceSupervisor(_) => moved(h, replayApply(h.acc, caller, FixedCodeLen, slot, anchor, message))
      | RemoveKV(_) => moved(h, applied)
      | Forget(_) => moved(h, replayApply(h.acc, caller, FixedCodeLen, slot, anchor, message))
      | Solicit(p) => match p.target {
        | Service(_) => moved(h, replayApply(h.acc, caller, FixedCodeLen, slot, anchor, message))
        | _ => { ...h, safe: false }
      }
      | RequestCodeUpgrade(_) => moved(h, applied)
      | CleanUpBucketsUpTo(_) => moved(h, applied)
      // These actions use only zero-amount transfers to force real pool exhaustion.
      // They are excluded from the completed prefix by replayInterrupt.
      | _ => { ...h, safe: false }
    }
  }
  type HostPackage = { state: ParachainServiceState, foreign: ForeignServices,
    free: int, safe: bool, rejects: int, minGas: Option[int], creations: List[CreationEffect],
    solicited: Set[(ParaId, Hash, int)], pruned: List[(ParaId, Timeslot)] }
  pure def limitedPackage(h: HostPackage, wr: WorkResult, slot: int): HostPackage = match wr.result {
    | WorkErr => h
    | WorkOk(digest) => match digest {
      | Err(_) => { ...h, safe: false }
      | Ok(ok) => {
        val prefix = accumulateOkPrefix(h.state, ok, FixedCodeLen, slot)
        if (prefix.rejected) h else {
          val delta = hostCost(prefix.state) - hostCost(h.state)
          val start: HostAcc = { free: h.free - delta, safe: h.safe and delta <= h.free,
            rejects: h.rejects, minGas: h.minGas, creations: h.creations, acc: { state: prefix.state, foreign: h.foreign,
              jamDesignate: List(), jamAssigns: List(), jamSolicit: Set(), jamForget: Set(), logs: List() } }
          val out = ok.upwardMessages.foldl(start, (a, m) => limitedMessage(a, ok.paraId, slot, ok.lookupAnchor, m))
          val logged = out.acc.state.appendAccumulateLogs(ok.paraId, slot, out.acc.logs)
          val logDelta = hostCost(logged) - hostCost(out.acc.state)
          val logFits = logDelta <= out.free
          { ...h, state: if (logFits) logged else out.acc.state, foreign: out.acc.foreign,
            free: out.free - (if (logFits) logDelta else 0), safe: out.safe,
            rejects: out.rejects, minGas: out.minGas, creations: out.creations,
            solicited: ok.upwardMessages.foldl(h.solicited, (a, m) => match m {
              | Forget(p) => match p.target {
                | Parachain(target) => a.exclude(Set((target, p.hash, p.len))) | _ => a
              } | _ => a
            }), pruned: h.pruned.append((ok.paraId, ok.lookupAnchor)) }
        }
      }
    }
  }

  // Unlike the isolated storage profile, this runs on the general campaign's
  // current state, including pending assignments, provided code, and old logs.
  // Unsupported backstop sites are guarded, never approximated as successes.
  def hostPrefix(packages: List[WorkResult], slot: int, incoming: List[IncomingTransfer],
    limits: List[int], allowance: int): HostPackage = {
    val due = accumulateBlock(svc, slot, List(), solicitedSet, FixedCodeLen, List(), foreignServices)
    val arrived = applyIncomingTransfers(due.state, incoming)
    val free = allowance + incoming.foldl(0, (n, t) => n + t.amount) - hostCost(arrived) + hostCost(svc)
    val start: HostPackage = { state: arrived, foreign: due.foreign, free: free, safe: free >= 0,
      rejects: 0, minGas: None, creations: List(), solicited: solicitedSet, pruned: List() }
    range(0, packages.length()).foldl(start, (a, i) =>
      if (limits[i] < reportGas(packages[i])) a else limitedPackage(a, packages[i], slot))
  }
  action hostInvocation(packages: List[WorkResult], slot: int, incoming: List[IncomingTransfer],
    limits: List[int], allowance: int, interrupt: int): bool = {
    val end = if (interrupt < 0) packages.length() else interrupt
    val out = hostPrefix(packages.slice(0, end), slot, incoming, limits.slice(0, end), allowance)
    val due = accumulateBlock(svc, slot, List(), solicitedSet, FixedCodeLen, List(), foreignServices)
    all {
      out.safe,
      svc' = out.state, foreignServices' = out.foreign, prevSvc' = svc, now' = slot,
      solicitedSet' = out.solicited, jamStagingSet' = jamStagingSet,
      lastStepWorkResults' = packages, lastStepAssigns' = due.assigns,
      lastHeadRoot' = if (interrupt < 0) headCommitment(svc, out.state) else None,
      logPrunedBelow' = out.pruned.foldl(logPrunedBelow, (m, pair) => m.put(pair._1, pair._2)),
      replayIncoming' = incoming, replayGasLimits' = limits, replayInterrupt' = interrupt,
      replayDesignate' = List(), replayTransfers' = List(), replayCreations' = out.creations,
      replayMinAccGas' = out.minGas.optionGetOr(replayMinAccGas),
      replayHostBudget' = allowance, replayHostFree' = out.free, replayHostRejects' = out.rejects
    }
  }
}
