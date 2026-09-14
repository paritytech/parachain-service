// KV sequences evaluated by the pinned Quint model.
module kv {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"

  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"

  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"

  pure def valueOfLength(n: int): List[int] = range(0, n).foldl(List(), (acc, _) => acc.append(255))
  pure def writeKV(key: List[int], value: List[int]): UpwardMessage = SetKV({ key: key, value: value })
  pure def remove(para: ParaId, key: List[int]): UpwardMessage = RemoveKV({ paraId: para, key: key })
  action core(msgs: List[UpwardMessage]): bool = applyHappyPath(CoretimeParaId, { vchBytes: 1 }, msgs)
  action hub(msgs: List[UpwardMessage]): bool = applyHappyPath(AssetHubParaId, { vchBytes: 2 }, msgs)

  run overwrite_and_refund_works = {
    val key = List(1, 0, 255)
    init
      .then(core(List(writeKV(key, valueOfLength(63)))))
      .then(core(List(writeKV(key, valueOfLength(64)))))
      .then(expectThat(svc.paraInfo(CoretimeParaId).usedStateBalance ==
        prevSvc.paraInfo(CoretimeParaId).usedStateBalance + 2))
      .then(core(List(writeKV(key, List()))))
      .then(expectThat(svc.keyValueStorage.get((CoretimeParaId, key)) == List() and
        svc.paraInfo(CoretimeParaId).usedStateBalance ==
          prevSvc.paraInfo(CoretimeParaId).usedStateBalance - 65))
      .then(core(List(remove(CoretimeParaId, key), remove(CoretimeParaId, key))))
      .then(expectThat(svc.keyValueStorage == Map() and
        svc.paraInfo(CoretimeParaId).usedStateBalance ==
          prevSvc.paraInfo(CoretimeParaId).usedStateBalance - kvEntryFootprint(key.length(), 0)))
      .then(core(List(writeKV(List(), List()), remove(CoretimeParaId, List()), writeKV(List(), List(0, 255)))))
      .then(expectThat(svc.keyValueStorage.get((CoretimeParaId, List())) == List(0, 255)))
  }

  // Identical keys have independent owners. Coretime sees and deletes the
  // earlier WP's write, then Asset Hub recreates it in the following block.
  run delegated_removal_works = {
    val key = List(2)
    init.then(applyHappyPaths(List(
      (AssetHubParaId, { vchBytes: 2 }, List(writeKV(key, List(9)))),
      (CoretimeParaId, { vchBytes: 1 }, List(writeKV(key, List(8)), remove(AssetHubParaId, key)))
    )))
      .then(expectThat(svc.keyValueStorage == Map((CoretimeParaId, key) -> List(8))))
      .then(hub(List(writeKV(key, List(7)))))
      .then(hub(List(writeKV(key, List(6)), remove(CoretimeParaId, key))))
      .then(expectThat(svc.keyValueStorage == prevSvc.keyValueStorage and
        svc.paraInfo(AssetHubParaId).headData == prevSvc.paraInfo(AssetHubParaId).headData))
      .then(core(List(remove(AssetHubParaId, key), remove(CoretimeParaId, key))))
      .then(expectThat(svc.keyValueStorage == Map()))
  }

  run insufficient_balance_works = {
    val key = List(3)
    val other = List(4)
    init.then(core(List(writeKV(key, List(42)))))
      .then(core(List(ParachainSetStateBalance({ paraId: CoretimeParaId,
        newTotal: svc.paraInfo(CoretimeParaId).usedStateBalance }))))
      .then(core(List(writeKV(key, List(43, 44)), writeKV(other, List(45)))))
      .then(expectThat(all {
        svc.keyValueStorage == prevSvc.keyValueStorage,
        svc.paraInfo(CoretimeParaId).usedStateBalance == prevSvc.paraInfo(CoretimeParaId).usedStateBalance,
        svc.paraLog(CoretimeParaId).nth(svc.paraLog(CoretimeParaId).length() - 1)._2 == AccumulateLogEntry(List(
          InsufficientStateBalance(FromSetKV({ keyHash: listHash(key) })),
          InsufficientStateBalance(FromSetKV({ keyHash: listHash(other) }))))
      }))
      // Same-size replacement needs no headroom; deletion funds a later write.
      .then(core(List(writeKV(key, List(99)), remove(CoretimeParaId, key), writeKV(other, List(45)))))
      .then(expectThat(svc.keyValueStorage == Map((CoretimeParaId, other) -> List(45))))
  }

  // Both candidates use the pre-block parent: only the first write can land.
  run stale_candidate_works = {
    val key = List(5)
    init.then(applyHappyPaths(List(
      (CoretimeParaId, { vchBytes: 1 }, List(writeKV(key, List(1)))),
      (CoretimeParaId, { vchBytes: 1 }, List(writeKV(key, List(2))))
    )))
      .then(expectThat(svc.keyValueStorage == Map((CoretimeParaId, key) -> List(1))))
  }
}
