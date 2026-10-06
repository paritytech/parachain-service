module host_mixed {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import fuzz.* from "./fuzz"
  import gas_inputs.* from "./gas_inputs"
  import transfer_inputs.* from "./transfer_inputs"
  import validator_key_inputs.* from "./validator_key_inputs"
  import outgoing_inputs.* from "./outgoing_inputs"
  import service_upgrade_inputs.* from "./service_upgrade_inputs"
  pure val remove81 = RemoveKV({ paraId: AssetHubParaId, key: List(81) })
  pure val remove82 = RemoveKV({ paraId: AssetHubParaId, key: List(82) })
  pure def storeKV(key: int, len: int): UpwardMessage = SetKV({ key: List(key),
    value: range(0, len).foldl(List(), (a, _) => a.append(42)) })
  pure def solicit(hash: int, len: int): UpwardMessage =
    Solicit({ target: Parachain(AssetHubParaId), hash: { hashBytes: hash }, len: len })
  pure def forget(hash: int, len: int): UpwardMessage =
    Forget({ target: Parachain(AssetHubParaId), hash: { hashBytes: hash }, len: len })
  pure def code(phase: CodeUpgradePhase): UpwardMessage =
    RequestCodeUpgrade({ hash: { vchBytes: 777 }, len: FixedCodeLen, phase: phase })
  action normal(caller: ParaId, msgs: List[UpwardMessage]): bool =
    replayBlock(List(replayWork(svc, caller, svc.paraInfo(caller).headData,
      activeCodeCandidate(svc, caller), 0, now, 0, msgs)), now + 1)
  action supplied(hash: int, len: int): bool = all {
    provide({ hashBytes: hash }, len), noGas, noTransfers, noDesignation, noOutgoing, keepUpgradeGas
  }
  run gas_rejection_works = replayInit
    .then(hostBlockAt(1000, 2, 1, false, remove81, remove82))
    .then(hostBlockAt(1000, 2, 2, false, remove81, remove82))
  run checkpoint_recovery_works = replayInit
    .then(hostBlockAt(1000, 1, 4, true, forget(7, 1024), remove82))
    .then(normal(AssetHubParaId, List(storeKV(81, 1))))
  run due_assignment_recovery_works = replayInit
    .then(normal(CoretimeParaId, List(AssignCore({ core: 0, queue: List({ authBytes: 10 }),
      jamSlot: 2, newAssigner: None }))))
    .then(hostBlockAt(1000, 2, 4, true, remove81, remove82))
    .then(normal(AssetHubParaId, List(storeKV(81, 1))))
  run release_then_retry_works = replayInit
    .then(normal(AssetHubParaId, List(storeKV(82, 8192))))
    .then(hostBlockAt(1000, 0, 4, false, remove82, storeKV(81, 4096)))
    .then(normal(AssetHubParaId, List(remove81)))
  run forget_then_retry_works = replayInit
    .then(normal(AssetHubParaId, List(solicit(777, FixedCodeLen))))
    .then(hostBlockAt(1000, 0, 4, false, forget(777, FixedCodeLen), storeKV(81, 4096)))
    .then(normal(AssetHubParaId, List(remove81)))
  run code_upgrade_works = replayInit
    .then(normal(AssetHubParaId, List(solicit(777, FixedCodeLen))))
    .then(supplied(777, FixedCodeLen))
    .then(normal(AssetHubParaId, List(code(Announcement))))
    .then(hostBlockAt(1000, 1, 4, true, code(Apply), remove82))
    .then(normal(AssetHubParaId, List(storeKV(81, 1))))
  run service_upgrade_works = replayInit
    .then(normal(CoretimeParaId, List(ParachainSetStateBalance({ paraId: AssetHubParaId,
      newTotal: InitialAssetHubBalance + 3 * ServiceCodeLen }))))
    .then(normal(AssetHubParaId, List(solicit(9001, ServiceCodeLen))))
    .then(supplied(9001, ServiceCodeLen))
    .then(hostBlockAt(1000, 1, 4, true, upgrade({ hashBytes: 9001 }, 123456, 789), remove82))
    .then(normal(AssetHubParaId, List(storeKV(81, 1))))
  run queue_cleanup_works = replayInit
    .then(hostBlockAt(1000, 3, 4, false, CleanUpBucketsUpTo(0), remove82))
    .then(normal(AssetHubParaId, List(storeKV(81, 1))))
}
