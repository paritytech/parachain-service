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
      List({ result: result, authTrace: authTrace }), foreignServices)
    all {
      svc' = out.state,
      foreignServices' = out.foreign,
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
    init.then(logError(TooManyValidatorKeys, 7)).then(applyWork(WorkErr, 7))

  run too_many_upward_messages_works =
    init.then(logError(TooManyUpwardMessages, 7)).then(applyWork(WorkErr, 7))

  run restricted_host_function_works =
    init.then(logError(RestrictedHostFunction, 7)).then(applyWork(WorkErr, 7))

  run refine_output_too_large_works =
    init.then(logError(RefineOutputTooLarge, 7)).then(applyWork(WorkErr, 7))

  run missing_head_declaration_works =
    init.then(logError(MissingHeadDeclaration, 7)).then(applyWork(WorkErr, 7))

  run invalid_core_index_works =
    init.then(logError(InvalidCoreIndex, 7)).then(applyWork(WorkErr, 7))

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

  run invalid_code_hash_works =
    init.then(logError(InvalidCodeHash, 7)).then(applyWork(WorkErr, 7))

  run invalid_code_hash_unregistered_works =
    init.then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: InvalidCodeHash })), 300))

  run invalid_code_hash_auth_trace_boundaries_works =
    init.then(logError(InvalidCodeHash, 0)).then(logError(InvalidCodeHash, 255)).then(logError(InvalidCodeHash, 256)).then(logError(InvalidCodeHash, 257))

  run too_many_validator_keys_unregistered_works =
    init.then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: TooManyValidatorKeys })), 300))

  run too_many_validator_keys_auth_trace_boundaries_works =
    init.then(logError(TooManyValidatorKeys, 0)).then(logError(TooManyValidatorKeys, 255)).then(logError(TooManyValidatorKeys, 256)).then(logError(TooManyValidatorKeys, 257))

  run too_many_upward_messages_unregistered_works =
    init.then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: TooManyUpwardMessages })), 300))

  run too_many_upward_messages_auth_trace_boundaries_works =
    init.then(logError(TooManyUpwardMessages, 0)).then(logError(TooManyUpwardMessages, 255)).then(logError(TooManyUpwardMessages, 256)).then(logError(TooManyUpwardMessages, 257))

  run restricted_host_function_unregistered_works =
    init.then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: RestrictedHostFunction })), 300))

  run restricted_host_function_auth_trace_boundaries_works =
    init.then(logError(RestrictedHostFunction, 0)).then(logError(RestrictedHostFunction, 255)).then(logError(RestrictedHostFunction, 256)).then(logError(RestrictedHostFunction, 257))

  run refine_output_too_large_unregistered_works =
    init.then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: RefineOutputTooLarge })), 300))

  run refine_output_too_large_auth_trace_boundaries_works =
    init.then(logError(RefineOutputTooLarge, 0)).then(logError(RefineOutputTooLarge, 255)).then(logError(RefineOutputTooLarge, 256)).then(logError(RefineOutputTooLarge, 257))

  run missing_head_declaration_unregistered_works =
    init.then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: MissingHeadDeclaration })), 300))

  run missing_head_declaration_auth_trace_boundaries_works =
    init.then(logError(MissingHeadDeclaration, 0)).then(logError(MissingHeadDeclaration, 255)).then(logError(MissingHeadDeclaration, 256)).then(logError(MissingHeadDeclaration, 257))

  run opaque_unregistered_works =
    init.then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: Opaque(1024) })), 300))

  run repeated_error_works =
    init.then(logError(MissingHeadDeclaration, 7)).then(logError(MissingHeadDeclaration, 7)).then(applyWork(WorkErr, 300))

  run mixed_errors_preserve_log_works =
    init.then(logError(InvalidCodeHash, 7)).then(logError(TooManyValidatorKeys, 7)).then(logError(TooManyUpwardMessages, 7)).then(logError(RestrictedHostFunction, 7)).then(logError(RefineOutputTooLarge, 7)).then(logError(MissingHeadDeclaration, 7)).then(logError(Opaque(1024), 7)).then(applyWork(WorkErr, 0))

  run unregistered_error_preserves_other_log_works =
    init.then(logError(Opaque(42), 7)).then(applyWork(WorkOk(Err({ paraId: MkParaId(999), error: MissingHeadDeclaration })), 300))

  run work_error_empty_log_works =
    init.then(applyWork(WorkErr, 0)).then(applyWork(WorkErr, 300))

  run opaque_1023_auth_257_works =
    init.then(logError(Opaque(1023), 257)).then(applyWork(WorkErr, 7))

}
