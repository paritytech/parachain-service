// Malformed work surrounds a valid candidate in every generated block.
module byte_pipeline {
  import fuzz as base from "./fuzz"
  import combined as cases from "./combined"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  action replayInit = base::replayInit
  action replayStep = {
    nondet fault = Set("candidate_empty", "candidate_truncated", "candidate_trailing",
      "pvf_empty", "pvf_magic", "pvf_truncated", "pvf_memory", "pvf_entry").oneOf()
    val good = cases::candidate(svc.paraInfo(CoretimeParaId).headData, 0)
    base::refinedBlock(List(cases::malformed(fault), good, cases::malformed(fault)), now + 1)
  }
}
