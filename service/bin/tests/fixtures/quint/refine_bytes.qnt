// Keep the existing host-call boundary campaign intact while sampling byte faults.
module byte_fuzz {
  import fuzz as base from "./refine_fuzz"
  action replayInit = base::replayInit
  action replayStep = {
    nondet fault = Set("valid", "candidate_empty", "candidate_truncated",
      "candidate_trailing", "pvf_empty", "pvf_magic", "pvf_truncated", "pvf_memory", "pvf_entry").oneOf()
    nondet available = Set(true, false).oneOf()
    base::invoke({ ...base::input(base::declarations), byteFault: fault, available: available })
  }
}
