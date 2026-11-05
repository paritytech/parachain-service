module authorization_fuzz {
  import authorization as campaign from "./authorization"
  action replayInit = campaign::authInit
  action replayStep = {
    val faults = Set("valid", "signature", "proof", "signed_context", "wrong_service",
      "para_count", "zero_collators", "zero_duration", "config_empty", "config_truncated",
      "config_trailing", "token_empty", "token_truncated", "token_trailing",
      "trace_empty", "trace_truncated", "trace_trailing")
    nondet first = faults.oneOf()
    nondet second = faults.oneOf()
    nondet mode = Set(0, 2, 4).oneOf()
    campaign::authorizeBlock(List(first, second), mode)
  }
}
