// Compose incoming transfers and work reports using the pinned model phases.
module invocation_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  import transfer_inputs.* from "./transfer_inputs"

  import validator_key_inputs.* from "./validator_key_inputs"

  import outgoing_inputs.* from "./outgoing_inputs"

  import service_upgrade_inputs.* from "./service_upgrade_inputs"

  import head_commitment.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/head_commitment"

  action replayInvocation(packages: List[WorkResult], slot: Timeslot,
    incoming: List[IncomingTransfer]): bool = {
    // Evaluate the pinned phases in runtime order, without inserting a second
    // invocation between incoming transfers and work reports.
    val prefix = accumulateBlock(svc, slot, List(), solicitedSet,
      FixedCodeLen, List(), foreignServices)
    val arrived = applyIncomingTransfers(prefix.state, incoming)
    val out = accumulateAll(arrived, solicitedSet, List(), FixedCodeLen,
      slot, packages, foreignServices)
    val effects = messageEffectsFrom(arrived, packages, slot)
    all {
      replayIncoming' = incoming,
      svc' = out.state,
      foreignServices' = out.foreign,
      now' = slot,
      replayDesignate' = out.jamDesignate,
      replayTransfers' = effects.transfers,
      replayMinAccGas' = effects.minAccGas.optionGetOr(replayMinAccGas),
      jamStagingSet' = if (out.jamDesignate == List()) jamStagingSet else out.jamDesignate,
      solicitedSet' = out.solicited,
      prevSvc' = svc,
      lastStepWorkResults' = packages,
      lastStepAssigns' = prefix.assigns.concat(out.jamAssigns),
      lastHeadRoot' = headCommitment(svc, out.state),
      logPrunedBelow' = out.logPruned.foldl(logPrunedBelow,
        (m, pair) => m.put(pair._1, pair._2))
    }
  }
}
