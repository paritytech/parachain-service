// Accumulate-side refine-error regressions. Work results are inputs; expected
// service states are computed by the pinned Quint accumulateBlock implementation.
module refine_errors {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"

  action applyWork(result: WorkExecResult, authTrace: int): bool = {
    val out = accumulateBlock(
      svc, now + 1, jamStagingSet, solicitedSet, FixedCodeLen,
      List({ result: result, authTrace: authTrace }))
    all {
      svc' = out.state,
      now' = now + 1,
      jamStagingSet' = out.staging,
      solicitedSet' = out.solicited,
      prevSvc' = svc,
      lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns,
      lastHeadRoot' = out.headRoot,
      logPrunedBelow' = logPrunedBelow
    }
  }

  action logError(error: RefineLog, authTrace: int): bool =
    applyWork(WorkOk(Err({ paraId: CoretimeParaId, error: error })), authTrace)

  run too_many_validator_keys_works =
    init.then(logError(SetValidatorKeysTooManyKeys, 7)).then(applyWork(WorkErr, 7))

  run too_many_upward_messages_works =
    init.then(logError(TooManyUpwardMessages, 7)).then(applyWork(WorkErr, 7))

  run restricted_host_function_works =
    init.then(logError(RestrictedHostFunction, 7)).then(applyWork(WorkErr, 7))

  run refine_output_too_large_works =
    init.then(logError(RefineOutputTooLarge, 7)).then(applyWork(WorkErr, 7))

  run missing_head_declaration_works =
    init.then(logError(MissingHeadDeclaration, 7)).then(applyWork(WorkErr, 7))

  run opaque_0_auth_0_works =
    init.then(logError(Opaque(0), 0)).then(applyWork(WorkErr, 7))

  run opaque_42_auth_7_works =
    init.then(logError(Opaque(42), 7)).then(applyWork(WorkErr, 7))

  run opaque_63_auth_255_works =
    init.then(logError(Opaque(63), 255)).then(applyWork(WorkErr, 7))

  run opaque_64_auth_256_works =
    init.then(logError(Opaque(64), 256)).then(applyWork(WorkErr, 7))

  run opaque_1024_auth_300_works =
    init.then(logError(Opaque(1024), 300)).then(applyWork(WorkErr, 7))

}
