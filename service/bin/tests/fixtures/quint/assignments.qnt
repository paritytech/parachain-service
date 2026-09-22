module assignments {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import lifecycle_inputs.* from "./lifecycle_inputs"
  pure def q(n: int): List[AuthorizerHash] = range(0, n).foldl(List(), (acc, i) => acc.append({ authBytes: i + 10 }))
  pure def msg(c: int, n: int, slot: int, owner: Option[ServiceId]): UpwardMessage =
    AssignCore({ core: c, queue: q(n), jamSlot: slot, newAssigner: owner })
  action core(msgs: List[UpwardMessage], gap: int): bool = lifecycleWork(CoretimeParaId, 1, msgs, gap)
  action empty(gap: int): bool = {
    val out = accumulateBlock(svc, now + gap, jamStagingSet, solicitedSet,
      FixedCodeLen, List(), foreignServices)
    all {
      svc' = out.state, now' = now + gap, prevSvc' = svc,
      jamStagingSet' = out.staging, solicitedSet' = out.solicited,
      lastStepWorkResults' = out.results, lastStepAssigns' = out.assigns,
      lastHeadRoot' = out.headRoot, foreignServices' = out.foreign,
      logPrunedBelow' = logPrunedBelow
    }
  }
  run inline_works = init
    .then(core(List(msg(0, 1, now, None), msg(1, 2, now + 1, None),
      msg(340, 80, now + 1, None)), 1))
    .then(expectThat(lastStepAssigns.length() == 3 and svc.pendingAssigns.keys() == Set()))

  run delayed_boundary_works = init
    .then(core(List(msg(2, 3, now + 3, None)), 1))
    .then(empty(1))
    .then(expectThat(lastStepAssigns == List()))
    .then(empty(1))
    .then(expectThat(lastStepAssigns.length() == 1))
    .then(empty(AuthorizerQueueLength - 1))
    .then(expectThat(lastStepAssigns == List()))
    .then(empty(1))
    .then(expectThat(lastStepAssigns.length() == 1))
    .then(empty(AuthorizerQueueLength + 1))

  run queue_lengths_works = init
    .then(core(List(msg(0, 3, now, None), msg(1, 40, now, None),
      msg(2, 79, now, None), msg(3, 80, now, None)), 1))
    .then(empty(AuthorizerQueueLength))
    .then(expectThat(svc.pendingAssigns.keys() == Set(0, 2)))

  run replacement_order_works = init
    .then(core(List(msg(2, 3, now + 3, None), msg(2, 5, now + 2, None)), 1))
    // Flush before work, then overwrite the same core twice inline.
    .then(core(List(msg(2, 7, now + 1, None), msg(2, 2, now, None)), 1))
    .then(expectThat(lastStepAssigns.length() == 3 and svc.pendingAssigns.keys() == Set()))
    .then(empty(AuthorizerQueueLength))

  run authorization_works = init
    .then(lifecycleWork(AssetHubParaId, 2, List(msg(2, 2, now, None)), 1))
    .then(expectThat(lastStepAssigns == List() and svc.pendingAssigns.keys() == Set()))
    .then(core(List(msg(2, 2, now, None)), 1))
    .then(expectThat(lastStepAssigns.length() == 1))

  run invalid_queue_works = init
    .then(core(List(msg(2, 3, now + 10, None)), 1))
    .then(core(List(msg(2, 0, now, None)), 1))
    .then(core(List(msg(2, 81, now, None)), 1))
    .then(core(List(msg(2, 3, now, Some(MkServiceId(7)))), 1))
    .then(expectThat(lastStepAssigns == List() and svc.pendingAssigns.get(2).queue == q(3)))
    .then(empty(10))

  run handoff_works = init
    .then(core(List(msg(0, 80, now, Some(MkServiceId(0))),
      msg(1, 80, now + 2, Some(MkServiceId(7))),
      msg(2, 80, now, Some(ParachainServiceId))), 1))
    .then(empty(1))
    .then(expectThat(lastStepAssigns.length() == 1 and svc.pendingAssigns.keys() == Set()))
    .then(core(List(msg(2, 1, now, None)), 1))

  run rejected_work_flush_works = init
    .then(core(List(msg(0, 3, now + 2, None)), 1))
    // Unauthorized work still runs the always-accumulate flush.
    .then(lifecycleWork(AssetHubParaId, 2, List(msg(1, 2, now, None)), 1))
    .then(expectThat(lastStepAssigns.length() == 1))
}
