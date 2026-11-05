// Independent checkpoint expectations for injected faults during funded flushes.
module due_flush {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  import assignment_inputs.* from "./assignment_inputs"
  import lifecycle_inputs.* from "./lifecycle_inputs"
  import transfer_inputs.* from "./transfer_inputs"

  import validator_key_inputs.* from "./validator_key_inputs"

  import outgoing_inputs.* from "./outgoing_inputs"

  import service_upgrade_inputs.* from "./service_upgrade_inputs"

  import invocation_inputs.* from "./invocation_inputs"

  import fuzz.* from "./fuzz"

  import gas_inputs.* from "./gas_inputs"

  var replayDueFlushBoundary: str
  action dueInit = all { replayInit, replayDueFlushBoundary' = "" }

  action schedule(count: int): bool = all { replayDueFlushBoundary' = "", replayBlock(List(replayWork(svc, CoretimeParaId, 0,
    activeCodeCandidate(svc, CoretimeParaId), 0, 0, 0,
    range(0, 3).foldl(List(), (a, core) => a.append(AssignCore({ core: core,
      queue: range(0, count).foldl(List(), (q, i) => q.append({ authBytes: 10 + core * 100 + i })),
      jamSlot: 2, newAssigner: None }))))), 1) }
  def work: List[WorkResult] = List(replayWork(svc, AssetHubParaId,
    svc.paraInfo(AssetHubParaId).headData, activeCodeCandidate(svc, AssetHubParaId),
    0, now + 1, 0, List(SetKV({ key: List(73), value: List(9) }))))
  // The initial flush has no checkpoint: roll back assignments and storage,
  // but preserve the scheduler's incoming credit. Retrying must fire all cores.
  action initialFault = all { replayDueFlushBoundary' = "initial:after:assign:0", preCheckpointInvocation(work, now + 1, List({
    source: MkServiceId(7), amount: 1000, toSupervisorBalance: false, memo: 1 })) }
  // No initial allowance; work commits before the final flush. Its assignment
  // effects are uncheckpointed and must roll back without losing the report.
  action finalFault = all { replayDueFlushBoundary' = "final:after:assign:0",
    budgetedInvocation(work, now + 1, List(), List(UnlimitedGas), 1) }
  action retry = all { replayDueFlushBoundary' = "", replayInvocation(List(), now + 1, List()) }

  run initial_tiling_works = dueInit.then(schedule(1)).then(initialFault).then(retry)
  run initial_rotating_works = dueInit.then(schedule(3)).then(initialFault).then(retry)
  run final_tiling_works = dueInit.then(schedule(1)).then(finalFault).then(retry)
  run final_rotating_works = dueInit.then(schedule(3)).then(finalFault).then(retry)
}
