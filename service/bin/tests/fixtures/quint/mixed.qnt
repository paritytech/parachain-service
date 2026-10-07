// Input generator for the supported replay profile. No expected states
// or transition sequences are prescribed here; semantics come from the pin.
module mixed {
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

  pure def arrival(amount: int, memo: int): IncomingTransfer = {
    source: MkServiceId(7), amount: amount, toSupervisorBalance: false, memo: memo
  }
  def hubWork(mode: int, msgs: List[UpwardMessage]): WorkResult =
    replayWork(svc, AssetHubParaId, svc.paraInfo(AssetHubParaId).headData,
      activeCodeCandidate(svc, AssetHubParaId), mode, now, 0, msgs)

  // Transfers occur after reports in the Rust operand list, but must be
  // credited and recorded first. The outgoing effect uses the credited state.
  run arrivals_and_reports_works = replayInit
    .then(replayInvocation(List(hubWork(0,
      List(TransferOut(transfer(0, 100, 1, 100))))), 1,
      List(arrival(1000, 1), arrival(2000, 2))))
    .then(replayInvocation(List(hubWork(0, List())), 2, List(arrival(0, 3))))

  // Rejected and skipped work must not lose the incoming phase's effects.
  run arrivals_and_rejected_reports_works = replayInit
    .then(replayInvocation(List(hubWork(7, List()),
      { result: WorkErr, authTrace: 7 }), 1, List(arrival(1000, 4))))
    .then(replayInvocation(List(hubWork(0, List())), 2, List()))

  // Cross a bucket boundary in one invocation, then append another batch
  // alongside work. Each invocation closes its tail bucket.
  run mixed_bucket_boundary_works = replayInit
    .then(replayInvocation(List(hubWork(0, List())), 1,
      range(0, MaxTransfersPerBucket + 1).foldl(List(),
        (acc, i) => acc.append(arrival(1000, i)))))
    .then(replayInvocation(List(hubWork(0, List())), 2,
      List(arrival(1000, 99))))
  // This payment fails without the incoming credit. It checks both the Rust
  // balance credit and the effect observer's starting state.
  run incoming_funds_outgoing_works = replayInit
    .then(replayInvocation(List(hubWork(0,
      List(TransferOut(transfer(0, TotalSupply, 9, 100))))), 1,
      List(arrival(TotalSupply, 9))))

  run due_assignment_and_arrivals_works = replayInit
    .then(replayInvocation(List(replayWork(svc, CoretimeParaId, 0,
      activeCodeCandidate(svc, CoretimeParaId), 0, 0, 0,
      List(AssignCore({ core: 0, queue: List({ authBytes: 10 }),
        jamSlot: 2, newAssigner: None })))), 1, List()))
    .then(replayInvocation(List(hubWork(0, List())), 2,
      List(arrival(1000, 10))))

}
