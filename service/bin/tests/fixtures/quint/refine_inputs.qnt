// Inputs retained independently of the model's predicted work results.
module refine_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"
  type RefineInput = {
    pkg: WorkPackage, calls: List[CodeCall], anchor: int,
    available: bool, missingPrefix: bool, skip: bool, byteFault: str, gasMode: str, gasSample: int
  }
  type RefinedWork = { input: RefineInput, work: WorkResult }
  // None is reserved for historical/accumulate-only fixture actions.
  // Some(List()) is an integrated invocation with no work packages.
  var replayRefineInputs: Option[List[RefineInput]]
  action noRefine = replayRefineInputs' = None
  pure def workResults(packages: List[RefinedWork]): List[WorkResult] =
    packages.foldl(List(), (acc, p) => acc.append(p.work))
  pure def refineInputs(packages: List[RefinedWork]): Option[List[RefineInput]] =
    Some(packages.foldl(List(), (acc, p) => acc.append(p.input)))
}
