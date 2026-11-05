// End-to-end scripts: model Refine predicts digests; model Accumulate predicts state.
module combined {
  import fuzz.* from "./fuzz"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import refine_inputs.* from "./refine_inputs"

  def candidate(parent: int, mode: int): RefinedWork =
    replayCandidate(svc, CoretimeParaId, parent, activeCodeCandidate(svc, CoretimeParaId),
      mode, now, 7, List(SetKV({ key: List(1), value: List(parent + 1) })))

  // Byte faults are outside the typed upstream Refine domain. The boundary
  // oracle predicts whole-work failure; upstream Accumulate handles WorkErr.
  def malformed(fault: str): RefinedWork = {
    val p = candidate(svc.paraInfo(CoretimeParaId).headData, 0)
    { input: { ...p.input, byteFault: fault },
      work: { ...p.work, result: WorkErr } }
  }
  run malformed_works = replayInit
    .then(refinedBlock(List(malformed("candidate_empty"), malformed("candidate_truncated"),
      malformed("candidate_trailing")), 1))
    .then(refinedBlock(List(malformed("pvf_empty"), malformed("pvf_magic"),
      malformed("pvf_truncated"), malformed("pvf_memory"), malformed("pvf_entry")), 2))
    .then(refinedBlock(List(candidate(0, 0)), 3))
    .then(refinedBlock(List(malformed("pvf_entry"), candidate(1, 0),
      malformed("pvf_memory")), 4))
    .then(refinedBlock(List(candidate(2, 0)), 5))

  run gas_cutoffs_works = replayInit
    .then(refinedBlock(List(gasCandidate("zero", 0, 0), gasCandidate("funded", 0, 0)), 1))
    .then(refinedBlock(List(gasCandidate("child", 0, 1), gasCandidate("funded", 0, 1)), 2))
    .then(refinedBlock(List(gasCandidate("cutoff", 0, 2), gasCandidate("cutoff", 250000, 2)), 3))
    .then(refinedBlock(List(gasCandidate("cutoff", 500000, 2), gasCandidate("cutoff", 750000, 2)), 4))
    .then(refinedBlock(List(gasCandidate("cutoff", 1000000, 2), gasCandidate("funded", 0, 2)), 5))
    .then(refinedBlock(List(gasCandidate("funded", 0, 3)), 6))

  run single_works = replayInit.then(refinedBlock(List(candidate(0, 0)), 1))
    .then(refinedBlock(List(candidate(1, 0)), 2))
  run ordered_batch_works = replayInit.then(refinedBlock(
    List(candidate(0, 0), candidate(1, 0), candidate(2, 0)), 1))
  run errors_works = replayInit.then(refinedBlock(
    List(candidate(0, 2), candidate(0, 4), candidate(0, 1)), 1))
    .then(refinedBlock(List(candidate(0, 0)), 2))
  run checkpoint_works = replayInit.then(checkpointBlockAt(1))
}
