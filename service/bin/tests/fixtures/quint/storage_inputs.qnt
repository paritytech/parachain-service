// Replay extension for the pinned host's storage deposit backstop. Restricted
// to heads, one-byte KV keys, and small incoming batches. No host call is mocked.
module storage_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  import head_commitment.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/head_commitment"
  import transfer_inputs.* from "./transfer_inputs"
  type StorageReason = KVWrite(int)
  type StorageLog = List[(Timeslot, List[StorageReason])]
  var replayStorageBudget: int
  var replayStorageFree: int
  var replayStorageLogs: StorageLog
  var replayStorageSizes: int -> int
  var replayFailedHeads: List[int]
  type Host = { state: ParachainServiceState, free: int, logs: StorageLog,
    sizes: int -> int, failed: List[int] }

  // SCALE lengths and JAM deposit parameters, independently of Rust encoders.
  pure def compact(n: int): int = if (n < 64) 1 else if (n < 16384) 2 else 4
  pure def headBytes(h: int): int = if (h == 0) 0 else 8
  pure def cost(key: int, value: int): int = 10 + 34 + key + value
  pure def oldCost(h: Host, key: int): int = if (h.sizes.keys().contains(key)) h.sizes.get(key) else 0
  pure def fits(h: Host, key: int, footprint: int): bool = footprint - oldCost(h, key) <= h.free
  pure def write(h: Host, key: int, footprint: int): Host =
    { ...h, free: h.free - footprint + oldCost(h, key), sizes: h.sizes.put(key, footprint) }
  pure def logSize(log: StorageLog): int = compact(log.length()) + log.foldl(0, (n, e) =>
    n + 4 + 1 + compact(e._2.length()) + e._2.foldl(0, (m, r) => m + match r {
      | KVWrite(_) => 34
    }))
  pure def saveLog(h: Host, log: StorageLog): Host = {
    val footprint = cost(5, logSize(log))
    if (fits(h, 0, footprint)) { ...write(h, 0, footprint), logs: log } else h
  }
  pure def appendLog(h: Host, slot: int, errors: List[StorageReason]): Host =
    if (errors == List()) h else saveLog(h, h.logs.append((slot, errors)))

  pure def arrivals(h: Host, incoming: List[IncomingTransfer], slot: int): Host = {
    val credited = { ...h, free: h.free + incoming.foldl(0, (n, t) => n + t.amount) }
    if (incoming == List()) credited else {
      val admitted = applyIncomingTransfers(h.state, incoming)
      val first = nextBucketId(h.state)
      val ids = range(first, nextBucketId(admitted))
      val written = ids.foldl({ host: credited, failed: false }, (a, id) => {
        val bucket = admitted.incomingTransfers.get(id)
        val bytes = compact(bucket.length()) + bucket.foldl(0, (n, t) =>
          n + 4 + compact(t.amount) + 1 + 128)
        val bucketCost = cost(9, bytes)
        if (a.failed or not(fits(a.host, 1000 + id, bucketCost))) { ...a, failed: true }
        else { ...a, host: write(a.host, 1000 + id, bucketCost) }
      })
      // Any failed bucket/endpoint write removes this invocation's new buckets.
      if (written.failed or not(fits(written.host, 1, cost(1, 20))))
        credited
      else { ...write(written.host, 1, cost(1, 20)), state: admitted }
    }
  }

  pure def report(h: Host, wr: WorkResult, index: int, slot: int): Host = match wr.result {
    | WorkErr => h
    | WorkOk(digest) => match digest {
      | Err(_) => h
      | Ok(ok) => {
        val prefix = accumulateOkPrefix(h.state, ok, FixedCodeLen, slot)
        if (prefix.rejected) h else {
          val pruned = if (h.logs == List()) h else
            saveLog(h, h.logs.select(e => e._1 >= ok.lookupAnchor))
          val delta = headBytes(ok.headData) - headBytes(h.state.paraInfo(ok.paraId).headData)
          val headFits = delta <= pruned.free
          val headed = if (headFits) { ...pruned, state: prefix.state, free: pruned.free - delta }
            else { ...pruned, failed: pruned.failed.append(index) }
          val done = ok.upwardMessages.foldl({ host: headed,
            errors: List() }, (a, msg) => match msg {
              | SetKV(kv) => {
                val key = 100 + kv.key[0]
                val footprint = cost(5 + kv.key.length(), kv.value.length())
                val applied = applySetKV(a.host.state, ok.paraId, kv.key, kv.value)
                if (applied.log != None or not(fits(a.host, key, footprint)))
                  { ...a, errors: a.errors.append(KVWrite(kv.key[0])) }
                else { ...a, host: { ...write(a.host, key, footprint), state: applied.state } }
              }
              | _ => a
            })
          appendLog(done.host, slot, done.errors)
        }
      }
    }
  }

  pure def storageWork(parent: int, newHead: int, key: int, len: int, anchor: int): WorkResult = {
    result: WorkOk(Ok({ paraId: AssetHubParaId, validationCode: { vchBytes: 2 },
      parentHeadHash: headHash(parent), headData: newHead, lookupAnchor: anchor,
      upwardMessages: if (len == 0) List() else List(SetKV({ key: List(key),
        value: range(0, len).foldl(List(), (a, _) => a.append(42)) })) })), authTrace: 0
  }
  action storageInit = all { init, noTransfers,
    replayStorageBudget' = 0, replayStorageFree' = 0, replayStorageLogs' = List(),
    replayStorageSizes' = Map(), replayFailedHeads' = List() }

  action storageBlock(budget: int, count: int, amount: int, len: int, newHead: int, chain: bool): bool = {
    val parent = svc.paraInfo(AssetHubParaId).headData
    val packages = List(storageWork(parent, newHead, 9, len, now),
      storageWork(if (chain) newHead else parent, newHead + 1, 10, 1, now))
    val incoming = range(0, count).foldl(List(), (a, i) => a.append({
      source: MkServiceId(7), amount: amount, toSupervisorBalance: false, memo: i }))
    val start: Host = { state: svc, free: budget, logs: replayStorageLogs,
      sizes: replayStorageSizes, failed: List() }
    val arrived = arrivals(start, incoming, now + 1)
    val out = range(0, packages.length()).foldl(arrived,
      (a, i) => report(a, packages[i], i, now + 1))
    all {
      svc' = out.state, prevSvc' = svc, now' = now + 1,
      lastStepWorkResults' = packages, lastStepAssigns' = List(),
      lastHeadRoot' = headCommitment(svc, out.state),
      solicitedSet' = solicitedSet, jamStagingSet' = jamStagingSet,
      foreignServices' = foreignServices, logPrunedBelow' = logPrunedBelow,
      replayIncoming' = incoming, replayStorageBudget' = budget,
      replayStorageFree' = out.free, replayStorageLogs' = out.logs,
      replayStorageSizes' = out.sizes, replayFailedHeads' = out.failed
    }
  }
}
