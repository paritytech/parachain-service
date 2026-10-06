// Compose incoming transfers and work reports using the pinned model phases.
module invocation_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  import service_inputs.* from "./service_inputs"
  import transfer_inputs.* from "./transfer_inputs"

  import validator_key_inputs.* from "./validator_key_inputs"

  import outgoing_inputs.* from "./outgoing_inputs"

  import service_upgrade_inputs.* from "./service_upgrade_inputs"

  import head_commitment.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/head_commitment"

  import gas_inputs.* from "./gas_inputs"

  import host_metadata.* from "./host_metadata"

  action replayInvocation(packages: List[WorkResult], slot: Timeslot,
    incoming: List[IncomingTransfer]): bool =
    budgetedInvocation(packages, slot, incoming, packages.foldl(List(), (acc, _) => acc.append(UnlimitedGas)), -1)

  action budgetedInvocation(packages: List[WorkResult], slot: Timeslot,
    incoming: List[IncomingTransfer], limits: List[int], interrupt: int): bool =
    interruptedInvocation(packages, slot, incoming, limits, interrupt, false)

  action interruptedInvocation(packages: List[WorkResult], slot: Timeslot,
    incoming: List[IncomingTransfer], limits: List[int], interrupt: int, panic: bool): bool = {
    val applied = range(0, packages.length()).foldl(List(), (acc, i) =>
      if ((interrupt < 0 or i < interrupt) and limits[i] >= reportGas(packages[i]))
        acc.append(packages[i]) else acc)

    // Evaluate the pinned phases in runtime order, without inserting a second
    // invocation between incoming transfers and work reports.
    val prefix = accumulateBlock(svc, slot, List(), solicitedSet,
      FixedCodeLen, List(), foreignServices)
    val arrived = applyIncomingTransfers(prefix.state, incoming)
    val out = replayAll(arrived, solicitedSet, List(), FixedCodeLen,
      slot, applied, foreignServices)
    val effects = messageEffectsFrom(arrived, applied, slot)
    all {
      noHostBudget,
      replayIncoming' = incoming,
      replayGasLimits' = limits,
      replayInterrupt' = interrupt,
      replayPanic' = panic,
      svc' = out.state,
      foreignServices' = out.foreign,
      now' = slot,
      replayDesignate' = out.jamDesignate,
      replayTransfers' = effects.transfers,
      replayCreations' = effects.creations,
      replayMinAccGas' = effects.minAccGas.optionGetOr(replayMinAccGas),
      jamStagingSet' = if (out.jamDesignate == List()) jamStagingSet else out.jamDesignate,
      solicitedSet' = out.solicited,
      prevSvc' = svc,
      lastStepWorkResults' = packages,
      lastStepAssigns' = prefix.assigns.concat(out.jamAssigns),
      lastHeadRoot' = if (interrupt < 0) headCommitment(svc, out.state) else None,
      logPrunedBelow' = out.logPruned.foldl(logPrunedBelow,
        (m, pair) => m.put(pair._1, pair._2))
    }
  }
}
