// `lastAccumulate` watermark: only the first invocation of a timeslot that
// reaches its checkpoint flushes due assigns (§5.1).
module last_accumulate {
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
  import gas_inputs.* from "./gas_inputs"
  import fuzz.* from "./fuzz"

  pure def arrival(amount: int, memo: int): IncomingTransfer = {
    source: MkServiceId(7), amount: amount, toSupervisorBalance: false, memo: memo
  }
  def hubWork(value: int): WorkResult =
    replayWork(svc, AssetHubParaId, svc.paraInfo(AssetHubParaId).headData,
      activeCodeCandidate(svc, AssetHubParaId), 0, now, 0,
      List(SetKV({ key: List(73), value: List(value) })))

  // Core 0 becomes due at slot 2, and core 1 at slot 3.
  action schedule = replayBlock(List(replayWork(svc, CoretimeParaId, 0,
    activeCodeCandidate(svc, CoretimeParaId), 0, 0, 0,
    List(
      AssignCore({ core: 0, queue: List({ authBytes: 10 }), jamSlot: 2, newAssigner: None }),
      AssignCore({ core: 1, queue: List({ authBytes: 11 }), jamSlot: 3, newAssigner: None })
    ))), 1)

  // A first invocation at slot 0 still counts: `None` is not `Some(0)`.
  run slot_zero_works = replayInit
    .then(replayBlock(List(hubWork(1)), 0))
    .then(replayBlock(List(hubWork(2)), 0))
    .then(replayBlock(List(hubWork(3)), 1))

  run same_slot_works = replayInit.then(schedule)
    .then(replayBlock(List(hubWork(1)), 2))
    .then(replayBlock(List(hubWork(2)), 2))
    .then(replayBlock(List(hubWork(3)), 3))

  run incoming_same_slot_works = replayInit.then(schedule)
    .then(replayInvocation(List(), 2, List(arrival(1000, 1))))
    .then(replayInvocation(List(), 2, List(arrival(1000, 2))))
    .then(replayInvocation(List(hubWork(1)), 3, List(arrival(1000, 3))))

  // A rollback before the first checkpoint leaves the watermark behind, so the
  // next invocation in that slot is still the always-accumulate one.
  run incoming_after_rollback_works = replayInit.then(schedule)
    .then(preCheckpointInvocation(List(hubWork(1)), 2, List(arrival(1000, 1))))
    .then(replayInvocation(List(), 2, List(arrival(1000, 2))))
    .then(replayInvocation(List(), 2, List(arrival(1000, 3))))
}
