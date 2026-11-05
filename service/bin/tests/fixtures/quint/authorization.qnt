// Admission oracle for a bounded fault domain; cryptography executes in the real PVM.
module authorization {
  import fuzz.* from "./fuzz"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import refine_inputs.* from "./refine_inputs"

  type AuthorizedInput = { input: RefineInput, fault: str, accepted: bool }
  var replayAuthorizationInputs: List[AuthorizedInput]
  // -1 identifies the fixed collator's exact 32-byte trace in this profile.
  // Other replay profiles retain their nonnegative length-proxy mapping.
  pure val CollatorTrace = -1
  pure def badTrace(fault: str): bool =
    Set("trace_empty", "trace_truncated", "trace_trailing").contains(fault)
  pure def admitted(fault: str): bool = fault == "valid" or badTrace(fault)

  def candidate(mode: int): RefinedWork =
    replayCandidate(svc, CoretimeParaId, svc.paraInfo(CoretimeParaId).headData,
      activeCodeCandidate(svc, CoretimeParaId), mode, now, CollatorTrace,
      List(SetKV({ key: List(1), value: List(7) })))

  action authInit = all { replayInit, replayAuthorizationInputs' = List() }
  action authorizeBlock(faults: List[str], mode: int): bool = {
    val c = candidate(mode)
    val inputs = faults.foldl(List(), (acc, f) => acc.append({ input: c.input, fault: f, accepted: admitted(f) }))
    val packages = faults.foldl(List(), (acc, f) => if (not(admitted(f))) acc else
      acc.append(if (badTrace(f)) { input: c.input, work: { result: WorkErr, authTrace: CollatorTrace } } else c))
    all {
      replayAuthorizationInputs' = inputs,
      refinedBlock(packages, now + 1),
    }
  }

  run accepted_works = authInit.then(authorizeBlock(List("valid"), 0))
    .then(authorizeBlock(List("valid"), 2))
    .then(authorizeBlock(List("valid"), 4))
    .then(authorizeBlock(List("valid"), 0))
  run rejected_works = authInit.then(authorizeBlock(List(
    "signature", "proof", "signed_context", "wrong_service", "para_count",
    "zero_collators", "zero_duration", "config_empty", "config_truncated",
    "config_trailing", "token_empty", "token_truncated", "token_trailing"), 0))
    .then(authorizeBlock(List("signature", "valid", "token_trailing"), 0))
  run malformed_trace_works = authInit.then(authorizeBlock(List(
    "trace_empty", "trace_truncated", "trace_trailing"), 0))
    .then(authorizeBlock(List("valid"), 0))
}
