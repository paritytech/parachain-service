// Each randomly rotated cycle covers every mode/sample combination.
module gas_pipeline {
  import fuzz as base from "./fuzz"
  var gasPhase: int
  action replayInit = {
    nondet start = 0.to(14).oneOf()
    all { base::replayInit, gasPhase' = start }
  }
  action replayStep = all {
    gasPhase' = (gasPhase + 1) % 15,
    base::refineGasBlockAt(List("zero", "child", "cutoff")[gasPhase % 3],
      (gasPhase / 3) * 250000)
  }
}
