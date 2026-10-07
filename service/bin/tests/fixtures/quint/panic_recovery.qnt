// Input generator for the supported replay profile. No expected states
// or transition sequences are prescribed here; semantics come from the pin.
module panic_recovery {
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

  import gas_inputs.* from "./gas_inputs"

  run first_checkpoint_works = replayInit.then(panicBlockAt(0, UnlimitedGas))
    .then(replayBlock(List(hubWork(0, List(SetKV({ key: List(73), value: List(9) })))), 2))
  run middle_checkpoint_works = replayInit.then(panicBlockAt(1, UnlimitedGas))
    .then(replayBlock(List(hubWork(0, List(SetKV({ key: List(73), value: List(9) })))), 2))
  run last_checkpoint_works = replayInit.then(panicBlockAt(2, UnlimitedGas))
    .then(replayBlock(List(hubWork(0, List(SetKV({ key: List(73), value: List(9) })))), 2))
  // Decoding happens before the report gas gate, even with zero declared gas.
  run zero_report_gas_works = replayInit.then(panicBlockAt(1, 0))
    .then(replayBlock(List(hubWork(0, List())), 2))
  run repeated_panic_works = replayInit.then(panicBlockAt(1, UnlimitedGas))
    .then(panicBlockAt(2, UnlimitedGas)).then(replayBlock(List(hubWork(0, List())), 3))
  run assignment_checkpoint_works = replayInit
    .then(replayBlock(List(replayWork(svc, CoretimeParaId, 0,
      activeCodeCandidate(svc, CoretimeParaId), 0, 0, 0,
      List(AssignCore({ core: 0, queue: List({ authBytes: 10 }),
        jamSlot: 2, newAssigner: None })))), 1))
    .then(panicBlockAt(0, UnlimitedGas))
    .then(replayBlock(List(hubWork(0, List())), 3))
}
