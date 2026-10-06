// Independent SCALE/JAM footprint arithmetic for the mixed storage oracle.
// Absolute account/code overhead cancels: only differences are used.
module host_sizes {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  pure def scaleCompact(n: int): int = if (n < 64) 1 else if (n < 16384) 2
    else if (n < 1073741824) 4 else if (n < 4294967296) 5
    else if (n < 1099511627776) 6 else if (n < 281474976710656) 7
    else if (n < 72057594037927936) 8 else 9
  pure def storeCost(key: int, bytes: int): int = 44 + key + bytes
  pure def paraBytes(pi: ParaInfo): int = 1 + (if (pi.headData == 0) 0 else 8)
    + (if (pi.validationCode == None) 1 else 37)
    + (if (pi.announcedUpgrade == None) 1 else 37)
    + scaleCompact(pi.totalStateBalance) + scaleCompact(pi.usedStateBalance) + 1
  pure def accBytes(log: AccumulateLog): int = 1 + match log {
    | InsufficientStateBalance(reason) => 1 + match reason {
      | FromSolicit(p) => 32 + scaleCompact(p.len)
      | FromSetKV(_) => 32
    }
    | StateBalanceUpdateRejected(p) => 4 + scaleCompact(p.attempted) + 1 + match p.reason {
      | BelowUsed(b) => scaleCompact(b.currentTotal) + scaleCompact(b.currentUsed)
      | _ => 0
    }
    | DesignateRejected(p) => scaleCompact(p.len)
    | ServiceUpgradePreimageMissing(_) => 32
    | CodeUpgradeNotAvailable(p) => 32 + scaleCompact(p.len)
    | CodeUpgradeNotAnnounced(p) => 32 + scaleCompact(p.len)
    | CanNotRemoveCode(p) => 32 + scaleCompact(p.len)
    | CoreNotAssignable(_) => 2
    | TransferFailed(p) => scaleCompact(p.id) + 1
    | ForgetAgainAt(p) => 32 + scaleCompact(p.len) + 4
    | ServiceStoreFailed(_) => 5
    | ServiceSolicitFailed(_) => 5
    | ServiceEjectFailed(_) => 5
    | ServiceSupervisorFailed(_) => 5
    | ServiceCreation(p) => scaleCompact(p.id) + 1 + match p.result {
      | Created(_) => 4 | _ => 0
    }
    | _ => 0
  }
  pure def entryBytes(entry: LogEntry): int = 1 + match entry {
    | RefineLogEntry(p) => 1 + (match p.error {
      | Opaque(n) => scaleCompact(n) + n | _ => 0
    }) + scaleCompact(p.authTrace) + p.authTrace
    | AccumulateLogEntry(logs) => scaleCompact(logs.length()) + logs.foldl(0, (n, l) => n + accBytes(l))
  }
  pure def logBytes(log: List[(Timeslot, LogEntry)]): int =
    scaleCompact(log.length()) + log.foldl(0, (n, e) => n + 4 + entryBytes(e._2))
  pure def hostCost(s: ParachainServiceState): int = {
    val paras = s.parachains.keys().fold(0, (n, p) => n + storeCost(5, paraBytes(s.paraInfo(p))))
    val kv = s.keyValueStorage.keys().fold(0, (n, k) =>
      n + storeCost(5 + k._2.length(), s.keyValueStorage.get(k).length()))
    val logs = s.parachainLog.keys().fold(0, (n, p) => n + storeCost(5, logBytes(s.paraLog(p))))
    val registry = s.preimageRegistry.keys().fold(0, (n, k) => {
      val count = s.preimageRegistry.get(k).referencers.size()
      n + storeCost(37, scaleCompact(count) + 4 * count)
    })
    val preimages = s.preimageStatus.keys().fold(0, (n, k) => n + 20 + 81 + k._2)
    val buckets = s.incomingTransfers.keys().fold(0, (n, id) => {
      val entries = s.incomingTransfers.get(id)
      n + storeCost(9, scaleCompact(entries.length()) + entries.foldl(0, (b, t) =>
        b + 4 + scaleCompact(t.amount) + 1 + 128))
    })
    val endpoints = if (s.incomingTransferBuckets == None) 0 else storeCost(1, 20)
    val assigns = s.pendingAssigns.keys().fold(0, (n, c) => {
      val entry = s.pendingAssigns.get(c)
      n + storeCost(3, scaleCompact(entry.queue.length()) + 32 * entry.queue.length()
        + (if (entry.assigner == None) 1 else 5))
    })
    // Due flushing retains the dirty-index key even when the list becomes empty.
    val dirty = storeCost(1, scaleCompact(s.pendingAssignCores.keys().size()) + 6 * s.pendingAssignCores.keys().size())
    paras + kv + logs + registry + preimages + buckets + endpoints + assigns + dirty
  }
}
