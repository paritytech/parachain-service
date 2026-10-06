// Replay effects are evaluated by the pinned model, including same-set designation.
module outgoing_replay {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  var replayDesignate: List[ValidatorKey]
  action check(ok: bool): bool = all { expectThat(ok), replayDesignate' = replayDesignate, replayTransfers' = replayTransfers }
  import outgoing_inputs.* from "./outgoing_inputs"
  action start = all { outgoingInit, replayDesignate' = List() }
  def work(p: ParaId, parent: int, msgs: List[UpwardMessage]): WorkResult = {
    val h = if (p == AssetHubParaId) 2 else 1
    val pkg: WorkPackage = { authorizedParas: List(p),
      items: List({ validationCode: { hash: { vchBytes: h }, len: FixedCodeLen } }) }
    val calls = List(SetParentHeadHash(headHash(parent)), SetHead(parent + 1)).concat(sends(msgs))
    { result: refine(svc, pkg, false, calls, now)[0], authTrace: 0 }
  }
  action block(packages: List[WorkResult]): bool = {
    // With an empty incoming JAM set, out.staging is exactly the last
    // successful designation (or empty for no effect), even for identical sets.
    val out = accumulateBlock(svc, now + 1, List(), solicitedSet,
      FixedCodeLen, packages, foreignServices)
    all {
      svc' = out.state, now' = now + 1, prevSvc' = svc,
      replayDesignate' = out.staging,
      replayTransfers' = outgoingEffects(packages, now + 1),
      jamStagingSet' = if (out.staging == List()) jamStagingSet else out.staging,
      solicitedSet' = out.solicited, lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns, lastHeadRoot' = out.headRoot,
      foreignServices' = out.foreign,
      logPrunedBelow' = out.logPruned.foldl(logPrunedBelow, (m, pair) => m.put(pair._1, pair._2))
    }
  }
  action hub(m: UpwardMessage): bool = block(List(work(AssetHubParaId,
    svc.paraInfo(AssetHubParaId).headData, List(m))))

  run deferred_works = start
    .then(hub(TransferOut(transfer(0, 123, 1, 100))))
    .then(hub(TransferOut({ ...transfer(7, 456, 2, 200), source: Some(ParachainServiceId) })))
    .then(check(svc.jamAccount.balance == TotalSupply - 579 and replayTransfers.length() == 1))
    .then(block(List()))
  run zero_amount_works = start
    .then(hub(TransferOut(transfer(0, 0, 0, 100))))
    .then(check(replayTransfers.length() == 1 and svc.jamAccount.balance == TotalSupply))
  run refusal_order_works = start
    .then(hub(TransferOut({ ...transfer(99, 100, 1, 0), source: Some(MkServiceId(98)) })))
    .then(hub(TransferOut({ ...transfer(99, 100, 2, 0), source: Some(MkServiceId(7)) })))
    .then(hub(TransferOut({ ...transfer(0, 100, 3, 0), source: Some(MkServiceId(7)) })))
    .then(hub(TransferOut({ ...transfer(0, 100, 4, 0), deferred: None })))
    .then(hub(TransferOut({ ...transfer(0, 100, 5, 99), sourceSupervisorBalance: true })))
    .then(hub(TransferOut({ ...transfer(0, 100, 6, 100), sourceSupervisorBalance: true })))
    .then(hub(TransferOut(transfer(0, TotalSupply, 7, 100))))
    .then(check(replayTransfers == List() and svc.jamAccount.balance == TotalSupply))
  run gas_boundary_works = start
    .then(hub(TransferOut(transfer(7, 1, 1, 199))))
    .then(check(replayTransfers == List()))
    .then(hub(TransferOut(transfer(7, 1, 2, 200))))
    .then(hub(TransferOut(transfer(7, 1, 3, 201))))
    .then(check(svc.jamAccount.balance == TotalSupply - 2))
  run ordered_repeated_ids_works = start
    .then(block(List(work(AssetHubParaId, 0, List(
      TransferOut(transfer(0, 11, 7, 100)), TransferOut(transfer(7, 22, 7, 199)),
      TransferOut(transfer(7, 33, 7, 200)), TransferOut(transfer(0, 0, 7, 101)))),
      work(AssetHubParaId, 1, List(TransferOut(transfer(7, 44, 7, 201)))))))
    .then(check(replayTransfers.length() == 4 and svc.jamAccount.balance == TotalSupply - 88))
  run skipped_work_works = start
    .then(block(List(work(CoretimeParaId, 0, List(TransferOut(transfer(0, 1, 1, 100)))),
      work(AssetHubParaId, 99, List(TransferOut(transfer(0, 2, 2, 100)))),
      { result: WorkErr, authTrace: 0 })))
    .then(check(replayTransfers == List() and svc.jamAccount.balance == TotalSupply))
    .then(hub(TransferOut(transfer(0, 3, 3, 100))))
  run balance_carry_works = start
    .then(hub(TransferOut(transfer(0, TotalSupply - 1000000000, 1, 100))))
    .then(hub(TransferOut(transfer(7, 2000000000, 2, 200))))
    .then(check(replayTransfers == List() and svc.jamAccount.balance == 1000000000))
    .then(hub(TransferOut(transfer(7, 123, 3, 200))))
    .then(check(svc.jamAccount.balance == 999999877))
}
