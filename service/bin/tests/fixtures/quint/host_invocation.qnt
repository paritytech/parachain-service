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
  import preimage_history.* from "./preimage_history"
  type HostAcc = { acc: AccumulateOutcome, free: int, safe: bool,
    rejects: int, minGas: Option[int], creations: List[CreationEffect], transfers: List[TransferOutArgs] }
  pure def moved(h: HostAcc, acc: AccumulateOutcome): HostAcc = {
    val delta = hostCost(acc.state) - hostCost(h.acc.state)
      + h.acc.state.jamAccount.balance - acc.state.jamAccount.balance
    { ...h, acc: { ...acc, preimageOps: acc.preimageOps.concat(releasedReferences(h.acc.state, acc.state)) },
      free: h.free - delta, safe: h.safe and delta <= h.free }
  }
  pure def limitedMessage(h: HostAcc, caller: ParaId, slot: int, anchor: int,
    message: UpwardMessage): HostAcc = {
    val applied = applyUpwardMessage(h.acc, caller, FixedCodeLen, slot, anchor, message)
    match message {
      | TransferOut(t) => {
        // Substitute only the threshold input to the pinned transfer oracle.
        // hostPrefix supplies actual free balance from independent footprint
        // deltas; account/code overhead cancels against the injected allowance.
        // Deferred self-credit is delivered only after the invocation.
        val supported = t.source.optionGetOr(ParachainServiceId) == ParachainServiceId
          and t.deferred != None
          and not(t.sourceSupervisorBalance) and not(t.destSupervisorBalance)
        val balance = serviceThreshold(h.acc.state) + h.free
        val state = { ...h.acc.state, jamAccount: { ...h.acc.state.jamAccount, balance: balance } }
        val out = applyUpwardMessage({ ...h.acc, state: state }, caller, FixedCodeLen, slot, anchor, message)
        val debit = if (t.dest == ParachainServiceId and out.logs == h.acc.logs) t.amount
          else balance - out.state.jamAccount.balance
        val restored = { ...out, state: { ...out.state,
          jamAccount: { ...h.acc.state.jamAccount, balance: h.acc.state.jamAccount.balance - debit } } }
        val nextHost = moved(h, restored)
        { ...nextHost, safe: nextHost.safe and supported,
          transfers: if (out.logs == h.acc.logs) h.transfers.append(t) else h.transfers }
      }
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
      // Other backstop sites require their own compatibility oracle.
      | _ => { ...h, safe: false }
    }
  }
  type HostPackage = { state: ParachainServiceState, foreign: ForeignServices,
    free: int, safe: bool, rejects: int, minGas: Option[int], creations: List[CreationEffect],
    transfers: List[TransferOutArgs], failedHeads: List[int], solicited: Set[(ParaId, Hash, int)], pruned: List[(ParaId, Timeslot)] }
  pure def limitedPackage(h: HostPackage, wr: WorkResult, slot: int, index: int): HostPackage = match wr.result {
    | WorkErr => h
    | WorkOk(digest) => match digest {
      | Err(_) => { ...h, safe: false }
      | Ok(ok) => {
        val prefix = accumulateOkPrefix(h.state, ok, FixedCodeLen, slot)
        if (prefix.rejected) h else {
          // Log pruning precedes the independent head write. Either write can
          // fail, and a refused head does not suppress the upward messages.
          val pruned = { ...h.state, parachainLog: prefix.state.parachainLog }
          val pruneCost = hostCost(pruned) - hostCost(h.state)
          val base = if (pruneCost <= h.free) pruned else h.state
          val free = h.free - hostCost(base) + hostCost(h.state)
          val headed = { ...base, parachains: prefix.state.parachains }
          val headCost = hostCost(headed) - hostCost(base)
          val headFits = headCost <= free
          val start: HostAcc = { free: free - (if (headFits) headCost else 0), safe: h.safe,
            rejects: h.rejects + (if (headFits) 0 else 1), minGas: h.minGas, creations: h.creations, transfers: h.transfers,
            acc: { state: if (headFits) headed else base, foreign: h.foreign,
              jamDesignate: List(), jamAssigns: List(),
              preimageOps: releasedReferences(h.state, if (headFits) headed else base), logs: List() } }
          val out = ok.upwardMessages.foldl(start, (a, m) => limitedMessage(a, ok.paraId, slot, ok.lookupAnchor, m))
          val logged = out.acc.state.appendAccumulateLogs(ok.paraId, slot, out.acc.logs)
          val logDelta = hostCost(logged) - hostCost(out.acc.state)
          val logFits = logDelta <= out.free
          { ...h, state: if (logFits) logged else out.acc.state, foreign: out.acc.foreign,
            free: out.free - (if (logFits) logDelta else 0), safe: out.safe,
            failedHeads: if (headFits) h.failedHeads else h.failedHeads.append(index),
            rejects: out.rejects, minGas: out.minGas, creations: out.creations, transfers: out.transfers,
            solicited: applyPreimageOps(h.solicited, out.acc.preimageOps), pruned: h.pruned.append((ok.paraId, ok.lookupAnchor)) }
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
    val proposed = applyIncomingTransfers(due.state, incoming)
    val credited = { ...due.state, jamAccount: proposed.jamAccount }
    val available = allowance + incoming.foldl(0, (n, t) => n + t.amount)
      - hostCost(due.state) + hostCost(svc)
    // Fresh bucket writes only grow; endpoint persistence follows them. Any
    // rejection removes all fresh buckets, retaining JAM's incoming credits.
    val queued = { ...credited, incomingTransfers: proposed.incomingTransfers,
      incomingTransferBuckets: proposed.incomingTransferBuckets }
    val queueCost = hostCost(queued) - hostCost(credited)
    val queueFits = queueCost <= available
    val queueState = if (queueFits) queued else credited
    val queueFree = available - (if (queueFits) queueCost else 0)
    // Beyond reserved capacity, reattribution is a separate ParaInfo write.
    val chargeCost = hostCost(proposed) - hostCost(queued)
    val arrived = if (queueFits and chargeCost <= queueFree) proposed else queueState
    val free = available - hostCost(arrived) + hostCost(credited)
    val start: HostPackage = { state: arrived, foreign: due.foreign, free: free, safe: available >= 0,
      rejects: if (queueFits) 0 else 1, minGas: None, creations: List(),
      transfers: List(), failedHeads: List(), solicited: solicitedSet, pruned: List() }
    range(0, packages.length()).foldl(start, (a, i) =>
      if (limits[i] < reportGas(packages[i])) a else limitedPackage(a, packages[i], slot, i))
  }
  action hostInvocation(packages: List[WorkResult], slot: int, incoming: List[IncomingTransfer],
    limits: List[int], allowance: int, interrupt: int): bool = {
    val end = if (interrupt < 0) packages.length() else interrupt
    val out = hostPrefix(packages.slice(0, end), slot, incoming, limits.slice(0, end), allowance)
    val due = accumulateBlock(svc, slot, List(), solicitedSet, FixedCodeLen, List(), foreignServices)
    all {
      out.safe,
      // Scheduler delivery is outside guest execution and checkpoint rollback.
      // Only the retained prefix's emitted records receive this later credit.
      svc' = { ...out.state, jamAccount: out.state.jamAccount.credit(
        out.transfers.foldl(0, (n, t) => n + (if (t.dest == ParachainServiceId) t.amount else 0)), false) },
      foreignServices' = out.foreign, prevSvc' = svc, now' = slot,
      solicitedSet' = out.solicited, jamStagingSet' = jamStagingSet,
      lastStepWorkResults' = packages, lastStepAssigns' = due.assigns,
      lastHeadRoot' = if (interrupt < 0) headCommitment(svc, out.state) else None,
      logPrunedBelow' = out.pruned.foldl(logPrunedBelow, (m, pair) => m.put(pair._1, pair._2)),
      replayIncoming' = incoming, replayGasLimits' = limits, replayInterrupt' = interrupt, replayPanic' = false,
      replayDesignate' = List(), replayTransfers' = out.transfers, replayCreations' = out.creations,
      replayMinAccGas' = out.minGas.optionGetOr(replayMinAccGas),
      replayHostFailedHeads' = out.failedHeads, replayHostBudget' = allowance, replayHostFree' = out.free, replayHostRejects' = out.rejects
    }
  }
}
