// Assignment operands only; pinned Refine and Accumulate determine outcomes.
module assignment_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import lifecycle_inputs.* from "./lifecycle_inputs"
  pure def assignmentMessages(slot: Timeslot): Set[UpwardMessage] = {
    val lengths = Set(0, 1, 2, 3, 5, AuthorizerQueueLength - 1,
      AuthorizerQueueLength, AuthorizerQueueLength + 1)
    Set(0, 1, CoreCount - 1).fold(Set(), (cores, c) => cores.union(
      lengths.fold(Set(), (queues, n) => {
        val q = range(0, n).foldl(List(), (acc, i) => acc.append({ authBytes: 10 + i }))
        Set(slot - 1, slot, slot + 1, slot + AuthorizerQueueLength).fold(queues,
          (msgs, due) => msgs.union(Set(
            AssignCore({ core: c, queue: q, jamSlot: due, newAssigner: None }),
            AssignCore({ core: c, queue: q, jamSlot: due, newAssigner: Some(MkServiceId(7)) })
          )))
      })))
  }
  // Keep successful Coretime work reachable despite the broad mixed-UMP pool.
  action assignmentBlock = {
    nondet caller = oneOf(Set(CoretimeParaId, AssetHubParaId))
    nondet gap = oneOf(Set(1, AuthorizerQueueLength - 1, AuthorizerQueueLength,
      AuthorizerQueueLength + 1))
    nondet first = oneOf(assignmentMessages(now + gap))
    nondet second = oneOf(assignmentMessages(now + gap))
    nondet count = oneOf(Set(0, 1, 2))
    val h = activeCodeHashOr(svc.paraInfo(caller), NoCodeHash).vchBytes
    lifecycleWork(caller, h, List(first, second).slice(0, count), gap)
  }
}
