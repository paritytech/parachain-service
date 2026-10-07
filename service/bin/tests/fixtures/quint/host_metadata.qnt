// A temporary real host balance, scoped to one invocation. -1 disables it.
module host_metadata {
  var replayHostFailedHeads: List[int]
  var replayHostBudget: int
  var replayHostFree: int
  var replayHostRejects: int
  action noHostBudget = all {
    replayHostFailedHeads' = List(), replayHostBudget' = -1, replayHostFree' = -1, replayHostRejects' = 0
  }
}
