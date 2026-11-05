// Mixed, overwritten and repeated host effects across report checkpoints.
module host_calls {
  import fuzz.* from "./fuzz"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import service_upgrade_inputs.* from "./service_upgrade_inputs"
  import refine_inputs.* from "./refine_inputs"
  import gas_inputs.* from "./gas_inputs"
  import transfer_inputs.* from "./transfer_inputs"
  import validator_key_inputs.* from "./validator_key_inputs"
  import outgoing_inputs.* from "./outgoing_inputs"

  def input(p: ParaId, parent: int, msgs: List[UpwardMessage]): RefinedWork =
    replayCandidate(svc, p, parent, activeCodeCandidate(svc, p), 0, now, 0, msgs)
  action fund = refinedBlock(List(input(CoretimeParaId, svc.paraInfo(CoretimeParaId).headData,
    List(ParachainSetStateBalance({ paraId: AssetHubParaId, newTotal: 100000000 })))), now + 1)
  action request = refinedBlock(List(input(AssetHubParaId, svc.paraInfo(AssetHubParaId).headData,
    List(Solicit({ target: Parachain(AssetHubParaId), hash: { hashBytes: 9001 }, len: ServiceCodeLen }),
      Solicit({ target: Parachain(AssetHubParaId), hash: { hashBytes: 9002 }, len: ServiceCodeLen })))), now + 1)
  action supplied(hash: int): bool = all {
    provide({ hashBytes: hash }, ServiceCodeLen), noGas, noTransfers, noDesignation, noOutgoing, keepUpgradeGas
  }
  action history(stop: int): bool = {
    val parent = svc.paraInfo(AssetHubParaId).headData
    val packages = range(0, 3).foldl(List(), (a, i) => {
      val designation = SetValidatorKeys({ keys: List(0, 1, 2, 3, 4, 5), isLast: true })
      val first = upgrade({ hashBytes: if (i == 2) 9002 else 9001 }, 33, 44)
      val last = upgrade({ hashBytes: 9002 }, 33, 44)
      a.append(input(AssetHubParaId, parent + i,
        if (i == 1) List(designation, first, last) else List(first, designation, last)))
    })
    if (stop == -2) refinedPreCheckpoint(packages, now + 1, List())
    else if (stop == -1) refinedBlock(packages, now + 1)
    else refinedInterrupted(packages, now + 1, List(), List(UnlimitedGas, UnlimitedGas, UnlimitedGas), stop, stop < 3)
  }
  run ordered_works = replayInit.then(fund).then(request).then(supplied(9001)).then(supplied(9002))
    .then(history(-1)).then(history(-1))
  run pre_checkpoint_works = replayInit.then(fund).then(request).then(supplied(9001)).then(supplied(9002))
    .then(history(-2)).then(history(-1))
  run first_checkpoint_works = replayInit.then(fund).then(request).then(supplied(9001)).then(supplied(9002))
    .then(history(0)).then(history(-1))
  run middle_checkpoint_works = replayInit.then(fund).then(request).then(supplied(9001)).then(supplied(9002))
    .then(history(1)).then(history(-1))
  run last_checkpoint_works = replayInit.then(fund).then(request).then(supplied(9001)).then(supplied(9002))
    .then(history(2)).then(history(-1))
  run final_checkpoint_works = replayInit.then(fund).then(request).then(supplied(9001)).then(supplied(9002))
    .then(history(3)).then(history(-1))
}
