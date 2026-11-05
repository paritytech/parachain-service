// Independent finite-budget/nontermination boundary oracle lives in refine_fuzz.
module gas_fuzz {
  import fuzz as base from "./refine_fuzz"
  action replayInit = base::replayInit
  action replayStep = {
    nondet mode = Set("zero", "funded", "child").oneOf()
    nondet available = Set(true, false).oneOf()
    base::invoke({ ...base::input(base::declarations), gasMode: mode, available: available })
  }
}
