// Ordered successful designation/upgrade effects from model transitions.
module host_call_inputs {
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import validator_keys.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/validator_keys"
  import foreign_services.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/foreign_services"
  type HostCall = DesignateCall(List[ValidatorKey])
    | UpgradeCall({ codeHash: Hash, minAccGas: int, minMemoGas: int })
  var replayHostCalls: List[HostCall]
  action noHostCalls = replayHostCalls' = List()
  action keepHostCalls = replayHostCalls' = replayHostCalls

  // Refusals append logs before log pruning/bounding. Designation's primitive
  // exposes an explicit call even when it repeats the already installed keys.
  pure def messageHostCalls(before: AccumulateOutcome, after: AccumulateOutcome,
    msg: UpwardMessage): List[HostCall] =
    if (after.logs != before.logs) List() else match msg {
      | SetValidatorKeys(k) => {
        val designated = applySetValidatorKeys(before.state, k.keys, k.isLast).designate
        if (designated == List()) List() else List(DesignateCall(designated))
      }
      | UpgradeService(u) => if (u.service == None)
          List(UpgradeCall({ codeHash: u.codeHash, minAccGas: u.minAccGas, minMemoGas: u.minMemoGas }))
        else List()
      | _ => List()
    }

  // Focused validator fixtures use the pin directly. The integrated campaign
  // collects the same per-message effects in its compatibility observer.
  pure def packageHostCalls(state: ParachainServiceState, foreign: ForeignServices,
    packages: List[WorkResult], slot: Timeslot): List[HostCall] = {
    val out = packages.foldl({ state: state, foreign: foreign, calls: List() }, (a, wr) => {
      val calls = match wr.result {
        | WorkOk(digest) => match digest {
          | Ok(ok) => {
          val pre = accumulateOkPrefix(a.state, ok, FixedCodeLen, slot)
          val base: AccumulateOutcome = { state: pre.state, foreign: a.foreign,
            jamDesignate: List(), preimageOps: List(), jamAssigns: List(), logs: pre.logs }
          if (pre.rejected) List() else ok.upwardMessages.foldl({ acc: base, calls: List() }, (m, msg) => {
            val applied = applyUpwardMessage(m.acc, ok.paraId, FixedCodeLen, slot, ok.lookupAnchor, msg)
            { acc: applied, calls: m.calls.concat(messageHostCalls(m.acc, applied, msg)) }
          }).calls
          }
          | _ => List()
        }
        | _ => List()
      }
      val afterPackage = accumulateOnePackage(a.state, wr, Set(), List(), FixedCodeLen, slot, a.foreign)
      { state: afterPackage.state, foreign: afterPackage.foreign, calls: a.calls.concat(calls) }
    })
    out.calls
  }
}
