// Transfer inputs and an effect observer built only from pinned model functions.
module outgoing_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import foreign_services.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/foreign_services"

  import service_inputs.* from "./service_inputs"
  var replayTransfers: List[TransferOutArgs]
  action noOutgoing = all { replayTransfers' = List(), noCreations }
  pure def destination(id: ServiceId, gas: int): ForeignService = {
    supervisor: id, supervisorCreated: 0, created: 0,
    requests: Map(), storage: Map(),
    account: { balance: 1000000000, supervisorBalance: 0, minMemoGas: gas }
  }
  action outgoingInit = all {
    svc' = initialState, prevSvc' = initialState, now' = 0,
    jamStagingSet' = List(), solicitedSet' = Set(), lastStepWorkResults' = List(),
    lastStepAssigns' = List(), lastHeadRoot' = None, logPrunedBelow' = Map(),
    foreignServices' = Map(MkServiceId(0) -> destination(MkServiceId(0), 100),
      MkServiceId(7) -> destination(MkServiceId(7), 200)), noOutgoing
  }
  pure def transfer(dest: int, amount: int, id: int, gas: int): TransferOutArgs = {
    source: None, dest: MkServiceId(dest), amount: amount, id: id,
    sourceSupervisorBalance: false, destSupervisorBalance: false,
    deferred: Some((id, gas))
  }
  // Deliberately stay away from the model's conservative threshold approximation.
  // Small payments and TotalSupply exercise success and definite overdraft.
  pure val outgoingMessages: Set[UpwardMessage] = Set(0, 7, 99).fold(Set(), (msgs, dest) =>
    Set(0, 100, TotalSupply).fold(msgs, (acc, amount) =>
      Set(99, 100, 199, 200).fold(acc, (out, gas) => out.union(Set(
        TransferOut(transfer(dest, amount, gas, gas)),
        TransferOut({ ...transfer(dest, amount, gas, gas), source: Some(ParachainServiceId) }),
        TransferOut({ ...transfer(dest, amount, gas, gas), source: Some(MkServiceId(7)) }),
        TransferOut({ ...transfer(dest, amount, gas, gas), source: Some(MkServiceId(99)) }),
        TransferOut({ ...transfer(dest, amount, gas, gas), sourceSupervisorBalance: true }),
        TransferOut({ ...transfer(dest, amount, gas, gas), deferred: None })
      )))))

  // Observe effects through pinned model functions; duplicate IDs and no-op
  // upgrades cannot be inferred from net balances or final code hashes.
  type ReplayEffects = { transfers: List[TransferOutArgs], minAccGas: Option[int], creations: List[CreationEffect] }
  pure val noEffects: ReplayEffects = { transfers: List(), minAccGas: None, creations: List() }
  def messageEffects(packages: List[WorkResult], slot: Timeslot): ReplayEffects = {
    val initial = accumulateBlock(svc, slot, jamStagingSet, solicitedSet,
      FixedCodeLen, List(), foreignServices)
    messageEffectsFrom(initial.state, packages, slot)
  }
  // Observe packages after the invocation's assignment/arrival phases.
  def messageEffectsFrom(state: ParachainServiceState,
    packages: List[WorkResult], slot: Timeslot): ReplayEffects = {
    val observed = packages.foldl({ state: state, foreign: foreignServices,
      solicited: solicitedSet, staging: jamStagingSet, effects: noEffects }, (a, wr) => {
      val emitted = match wr.result {
        | WorkErr => noEffects
        | WorkOk(digest) => match digest {
          | Err(_) => noEffects
          | Ok(ok) => {
            val pre = accumulateOkPrefix(a.state, ok, FixedCodeLen, slot)
            if (pre.rejected) noEffects else {
              val base: AccumulateOutcome = { state: pre.state, foreign: a.foreign,
                jamDesignate: a.staging, jamSolicit: pre.jamSolicit,
                jamForget: pre.jamForget, jamAssigns: List(), logs: pre.logs }
              val messages = ok.upwardMessages.foldl({ acc: base, effects: noEffects }, (m, msg) => {
                val applied = replayApply(m.acc, ok.paraId, FixedCodeLen, slot, ok.lookupAnchor, msg)
                // applyUpwardMessage appends a log for every refused transfer or
                // upgrade; logs are only bounded later, when the package ends.
                val success = applied.logs.length() == m.acc.logs.length()
                val effects = match msg {
                  | CreateService(c) => {
                    val fresh = applied.foreign.keys().exclude(m.acc.foreign.keys())
                    { ...m.effects, creations: fresh.fold(m.effects.creations, (list, id) => list.append({ service: id, args: c })) }
                  }
                  | TransferOut(t) => if (success and t.deferred != None)
                    { ...m.effects, transfers: m.effects.transfers.append(t) } else m.effects
                  | UpgradeService(u) => if (success)
                    { ...m.effects, minAccGas: Some(u.minAccGas) } else m.effects
                  | _ => m.effects
                }
                { acc: applied, effects: effects }
              })
              messages.effects
            }
          }
        }
      }
      val after = replayPackage(a.state, wr, a.solicited, a.staging,
        FixedCodeLen, slot, a.foreign)
      { state: after.state, foreign: after.foreign, solicited: after.solicited,
        staging: after.jamDesignate, effects: {
          transfers: a.effects.transfers.concat(emitted.transfers),
          creations: a.effects.creations.concat(emitted.creations),
          minAccGas: if (emitted.minAccGas == None) a.effects.minAccGas else emitted.minAccGas
        } }
    })
    observed.effects
  }
  def outgoingEffects(packages: List[WorkResult], slot: Timeslot): List[TransferOutArgs] =
    messageEffects(packages, slot).transfers
}
