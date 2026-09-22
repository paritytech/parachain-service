module balances {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import transfer_inputs.* from "./transfer_inputs"

  action start = all { init, noTransfers }
  action core(msgs: List[UpwardMessage]): bool = all {
    applyHappyPath(CoretimeParaId, { vchBytes: 1 }, msgs), noTransfers
  }
  action hub(msgs: List[UpwardMessage]): bool = all {
    applyHappyPath(AssetHubParaId, { vchBytes: 2 }, msgs), noTransfers
  }
  pure def total(para: ParaId, amount: int): UpwardMessage =
    ParachainSetStateBalance({ paraId: para, newTotal: amount })
  pure def transfer(amount: int, memo: int): IncomingTransfer =
    { from: MkServiceId(7), amount: amount, toSupervisorBalance: false, memo: memo }
  pure def batch(n: int, amount: int): List[IncomingTransfer] =
    range(0, n).foldl(List(), (acc, i) => acc.append(transfer(amount, i)))

  run balance_reservations_works = start
    .then(core(List(total(CoretimeParaId, svc.paraInfo(CoretimeParaId).usedStateBalance - 1))))
    .then(core(List(total(CoretimeParaId, svc.paraInfo(CoretimeParaId).usedStateBalance))))
    .then(core(List(
      SetKV({ key: List(1), value: List(1) }),
      Solicit({ target: Parachain(CoretimeParaId), hash: { hashBytes: 7 }, len: 1024 }),
      RequestCodeUpgrade({ hash: { vchBytes: 777 }, len: FixedCodeLen, phase: Announcement })
    )))
    .then(core(List(total(CoretimeParaId, InitialParaBalance),
      SetKV({ key: List(1), value: List(1) }),
      Solicit({ target: Parachain(CoretimeParaId), hash: { hashBytes: 7 }, len: 1024 }),
      RequestCodeUpgrade({ hash: { vchBytes: 777 }, len: FixedCodeLen, phase: Announcement })
    )))
    .then(core(List(total(CoretimeParaId, svc.paraInfo(CoretimeParaId).usedStateBalance),
      RemoveKV({ paraId: CoretimeParaId, key: List(1) }),
      SetKV({ key: List(2), value: List(2) })
    )))
    .then(hub(List(total(CoretimeParaId, 0))))

  // Each invocation closes its tail bucket. Cross both the bucket capacity
  // and the reservation, then exercise dropped and self-funded arrivals.
  run incoming_boundaries_works = start
    .then(recordIncoming(batch(MaxTransfersPerBucket + 1, 0)))
    .then(recordIncoming(batch(MaxIncomingTransfers - MaxTransfersPerBucket - 2, 0)))
    .then(core(List(total(AssetHubParaId, svc.paraInfo(AssetHubParaId).usedStateBalance))))
    .then(recordIncoming(List(transfer(0, 10),
      transfer(IncomingTransferEntryFootprint - 1, 11),
      transfer(IncomingTransferEntryFootprint, 12),
      transfer(IncomingTransferEntryFootprint + 1, 13))))
    .then(recordIncoming(List(transfer(0, 14))))
    .then(core(List(total(AssetHubParaId, svc.paraInfo(AssetHubParaId).usedStateBalance - 1))))
    .then(hub(List(SetKV({ key: List(1), value: List(1) }))))
}
