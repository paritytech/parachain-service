module host_mixed {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import host_invocation.* from "./host_invocation"
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
  run head_queue_rejection_works = replayInit
    .then(hostBlockAt(0, 1, 4, true, remove81, remove82))
    .then(hostBlockAt(187, 1, 4, false, remove81, remove82))
    .then(hostBlockAt(188, 1, 4, true, remove81, remove82))
    .then(hostBlockAt(253, 1, 4, true, remove81, remove82))
    .then(normal(AssetHubParaId, List(storeKV(81, 1))))
  run queue_capacity_recovery_works = replayInit
    .then(hostBlockTransfers(2000000, 1024, 0, 4, false, remove81, remove82))
    .then(hostBlockTransfers(0, 3, 1, 4, true, remove81, remove82))
    .then(hostBlockTransfers(0, 3, 300, 4, true, remove81, remove82))
    .then(normal(AssetHubParaId, List(CleanUpBucketsUpTo(10000))))
    .then(hostBlockTransfers(0, 3, 0, 4, true, remove81, remove82))
    .then(normal(AssetHubParaId, List(storeKV(81, 1))))
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
  // An unprovided solicitation raises the private charge without allocating a
  // gigabyte of bytes. The next KV write crosses SCALE's 2^30 boundary.
  action chargeBoundary = normal(AssetHubParaId, List(solicit(888,
    1073741823 - svc.paraInfo(AssetHubParaId).usedStateBalance - 187)))
  action fundCharge = normal(CoretimeParaId, List(ParachainSetStateBalance({
    paraId: AssetHubParaId, newTotal: 2147483648 })))
  action chargeWrite(allowance: int, len: int, stop: int): bool =
    hostInvocation(List(replayWork(svc, AssetHubParaId,
      svc.paraInfo(AssetHubParaId).headData, activeCodeCandidate(svc, AssetHubParaId),
      0, 0, 0, List(storeKV(83, len)))), now + 1, List(), List(UnlimitedGas), allowance, stop)

  run kv_charge_insert_works = replayInit
    .then(fundCharge).then(chargeBoundary)
    .then(chargeWrite(0, 1, -1))
    .then(chargeWrite(1, 1, -1))
    .then(chargeWrite(51, 1, -1))
    .then(chargeWrite(52, 1, -1))
    .then(normal(AssetHubParaId, List(RemoveKV({ paraId: AssetHubParaId, key: List(83) }))))
  run kv_charge_overwrite_works = replayInit
    .then(fundCharge).then(normal(AssetHubParaId, List(storeKV(83, 1))))
    .then(chargeBoundary)
    .then(chargeWrite(0, 2, -1))
    .then(chargeWrite(1, 2, -1))
    .then(chargeWrite(2, 2, -1))
    .then(chargeWrite(0, 1, -1))
  run kv_charge_recovery_works = replayInit
    .then(fundCharge).then(chargeBoundary)
    .then(chargeWrite(0, 1, 1))
    .then(chargeWrite(1, 1, 1))
    .then(chargeWrite(52, 1, 0))
    .then(chargeWrite(52, 1, 1))
    .then(normal(AssetHubParaId, List(RemoveKV({ paraId: AssetHubParaId, key: List(83) }))))

  // The model commits only the prefix selected before VM gas calibration.
  // stop=0 rolls back the message; stop=1 retains it at the final checkpoint.
  action recoverMessage(message: UpwardMessage, stop: int): bool =
    hostInvocation(List(replayWork(svc, AssetHubParaId,
      svc.paraInfo(AssetHubParaId).headData, activeCodeCandidate(svc, AssetHubParaId),
      0, now, 0, List(message))), now + 1, List(), List(UnlimitedGas), 2000000, stop)
  action chargedQueue = hostBlockTransfers(2000000, 1024, 0, 4, false, remove81, remove82)
    .then(hostBlockTransfers(2000000, 3, 300, 4, false, remove81, remove82))
  run cleanup_partial_recovery_works = replayInit.then(chargedQueue)
    .then(recoverMessage(CleanUpBucketsUpTo(0), 0))
    .then(recoverMessage(CleanUpBucketsUpTo(0), 1))
    .then(normal(AssetHubParaId, List(CleanUpBucketsUpTo(10000))))
  run cleanup_full_recovery_works = replayInit.then(chargedQueue)
    .then(recoverMessage(CleanUpBucketsUpTo(10000), 0))
    .then(recoverMessage(CleanUpBucketsUpTo(10000), 1))
    .then(hostBlockTransfers(2000000, 1, 0, 4, false, remove81, remove82))
  run remove_refund_recovery_works = replayInit
    .then(normal(AssetHubParaId, List(storeKV(81, 4096))))
    .then(recoverMessage(remove81, 0)).then(recoverMessage(remove81, 1))
    .then(normal(AssetHubParaId, List(storeKV(81, 4096))))
  run forget_refund_recovery_works = replayInit
    .then(normal(AssetHubParaId, List(solicit(777, FixedCodeLen))))
    .then(recoverMessage(forget(777, FixedCodeLen), 0))
    .then(recoverMessage(forget(777, FixedCodeLen), 1))
    .then(normal(AssetHubParaId, List(solicit(777, FixedCodeLen))))
  run code_apply_recovery_works = replayInit
    .then(normal(AssetHubParaId, List(solicit(777, FixedCodeLen))))
    .then(supplied(777, FixedCodeLen))
    .then(normal(AssetHubParaId, List(code(Announcement))))
    .then(recoverMessage(code(Apply), 0)).then(recoverMessage(code(Apply), 1))
    .then(normal(AssetHubParaId, List(forget(2, FixedCodeLen), storeKV(81, 1))))

}
