module lifecycle {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import lifecycle_inputs.* from "./lifecycle_inputs"
  pure val p = MkParaId(3)
  action core(msgs: List[UpwardMessage]): bool = lifecycleWork(CoretimeParaId, 1, msgs, 1)
  action work(msgs: List[UpwardMessage]): bool = lifecycleWork(p, 777, msgs, 1)
  action registered = core(List(fund(p, InitialParaBalance), setHead(p, 42), code(p, 777)))

  run registration_works = init
    .then(core(List(setHead(p, 42), code(p, 777), ParachainCleanUp(p))))
    .then(core(List(fund(p, baselineFor(p) - 1))))
    .then(core(List(fund(p, baselineFor(p)), code(p, 777))))
    .then(registered)
    .then(work(List()))
    // Repeated registration updates the allowance, preserving head and code.
    .then(core(List(fund(p, InitialParaBalance))))
    .then(expectThat(svc.paraInfo(p).headData == 43))

  run unauthorized_works = init
    .then(lifecycleWork(AssetHubParaId, 2,
      List(fund(p, InitialParaBalance), setHead(p, 42), code(p, 777), ParachainCleanUp(p)), 1))
    .then(expectThat(not(svc.isLive(p))))
    .then(registered)
    .then(lifecycleWork(AssetHubParaId, 2, List(ParachainCleanUp(p)), 1))
    .then(expectThat(svc.isLive(p)))

  run cleanup_storage_works = init.then(registered)
    .then(work(List(SetKV({ key: List(1), value: List(1) }),
      Solicit({ target: Parachain(p), hash: { hashBytes: 7 }, len: 1024 }),
      RequestCodeUpgrade({ hash: { vchBytes: 778 }, len: FixedCodeLen }))))
    .then(core(List(ParachainCleanUp(p))))
    .then(expectThat(svc.isLive(p) and not(svc.paraInfo(p).isDeregistering)))
    .then(work(List(RemoveKV({ paraId: p, key: List(1) }),
      Forget({ target: Parachain(p), hash: { hashBytes: 7 }, len: 1024 }))))
    .then(core(List(ParachainCleanUp(p))))
    .then(expectThat(not(svc.isLive(p))))
    .then(work(List()))
    .then(core(List(ParachainCleanUp(p))))
    .then(registered)
    .then(work(List()))

  run cleanup_retry_works = init.then(registered)
    .then(work(List(RequestCodeUpgrade({ hash: { vchBytes: 778 }, len: FixedCodeLen }))))
    .then(applyHappyPaths(List()))
    .then(provide({ hashBytes: 777 }, FixedCodeLen))
    .then(provide({ hashBytes: 778 }, FixedCodeLen))
    .then(core(List(ParachainCleanUp(p))))
    .then(expectThat(svc.paraInfo(p).isDeregistering))
    .then(core(List(fund(p, InitialParaBalance), setHead(p, 0), code(p, 778))))
    .then(work(List(SetKV({ key: List(1), value: List(1) }))))
    .then(core(List(ParachainCleanUp(p))))
    .then(lifecycleWork(CoretimeParaId, 1, List(ParachainCleanUp(p)), ExpungePeriod + 1))
    .then(expectThat(not(svc.isLive(p))))
    .then(registered)
    .then(work(List()))

  run forced_code_works = init.then(registered)
    .then(work(List(RequestCodeUpgrade({ hash: { vchBytes: 778 }, len: FixedCodeLen }))))
    .then(core(List(code(p, 778), code(p, 778), setHead(p, 9))))
    .then(expectThat(svc.paraInfo(p).pendingUpgrade == None))
    .then(lifecycleWork(p, 778, List(), 1))
}
