// Standalone Refine differential replay. The script is input, not an oracle:
// typed inputs use pinned Refine; wire faults use the explicit boundary oracle below.
module fuzz {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import refine.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/refine"

  type Input = {
    pkg: WorkPackage, missingPrefix: bool, available: bool,
    calls: List[CodeCall], anchor: int, byteFault: str, gasMode: str
  }
  var replayRefine: Option[Input]
  var replayRefineExpected: List[WorkExecResult]

  pure val candidate: ParachainCandidate = { validationCode: { hash: { vchBytes: 1 }, len: 1 } }
  pure def modelState(available: bool): ParachainServiceState =
    if (available) { ...emptyState,
      preimageRegistry: Map((({ hashBytes: 1 }, 1), { referencers: Set(CoretimeParaId) })) }
    else emptyState

  action replayInit = all { replayRefine' = None, replayRefineExpected' = List() }
  action invoke(input: Input): bool = all {
    replayRefine' = Some(input),
    // The typed upstream model cannot represent malformed wire bytes. This
    // boundary extension predicts abnormal exits independently of Rust (§4.1/4.2).
    // Lookup precedes executable parsing, but candidate decoding precedes lookup.
    replayRefineExpected' = if (input.gasMode == "zero" or
      (input.gasMode == "child" and input.available) or input.byteFault == "candidate_empty" or
      input.byteFault == "candidate_truncated" or input.byteFault == "candidate_trailing" or
      (input.available and input.byteFault != "valid")) List(WorkErr)
      else refine(modelState(input.available), input.pkg,
        input.missingPrefix, input.calls, input.anchor)
  }
  pure def input(calls: List[CodeCall]): Input = {
    pkg: { authorizedParas: List(CoretimeParaId), items: List(candidate) },
    missingPrefix: false, available: true, calls: calls, anchor: 0, byteFault: "valid", gasMode: "normal"
  }
  pure val declarations = List(SetParentHeadHash(headHash(0)), SetHead(1))
  pure val callMenu: Set[CodeCall] = Set(
    SetParentHeadHash(headHash(0)), SetParentHeadHash(headHash(7)),
    SetHead(0), SetHead(1), SetHead(7), SetHeadOversized,
    ReportError(0), ReportError(63), ReportError(64), ReportError(1024), Panic,
    Send(SetKV({ key: List(1), value: List(2) })),
    Send(SetKV({ key: List(), value: List(2) })),
    Send(RemoveKV({ paraId: CoretimeParaId, key: List() })),
    Send(ParachainCleanUp(AssetHubParaId)),
    Send(SetValidatorKeys({ keys: List(), isLast: false })),
    Send(AssignCore({ core: 65535, jamSlot: 0, queue: List(), newAssigner: None }))
  )
  action replayStep = {
    nondet para = Set(CoretimeParaId, AssetHubParaId, MkParaId(42)).oneOf()
    nondet available = Set(true, true, true, false).oneOf()
    nondet mode = 0.to(9).oneOf()
    nondet anchor = Set(0, 1, 100, 4294967295).oneOf()
    nondet a = callMenu.oneOf()
    nondet b = callMenu.oneOf()
    nondet c = callMenu.oneOf()
    nondet payloadLen = Set(63, 64, 4096, 40960, 40961).oneOf()
    nondet count = Set(1023, 1024, 1025).oneOf()
    val calls = if (mode == 0) declarations
      else if (mode == 1) List(a, b, c)
      else if (mode == 2) declarations.concat(List(a, b, c))
      else if (mode == 3) List(a).concat(declarations).concat(List(b, c))
      else if (mode == 4) declarations.concat(List(Send(SetKV({ key: List(1),
        value: range(0, payloadLen).foldl(List(), (acc, _) => acc.append(2)) }))))
      else if (mode == 5) declarations.concat(range(0, count).foldl(List(),
        (acc, _) => acc.append(Send(SetKV({ key: List(1), value: List(2) })))))
      else List(a, b).concat(declarations)
    val pkg: WorkPackage = {
      authorizedParas: if (mode == 8) List() else if (mode == 9) List(para, para) else List(para),
      items: if (mode == 9) List(candidate, candidate) else List(candidate)
    }
    invoke({ pkg: pkg, missingPrefix: mode == 7, available: available,
      calls: calls, anchor: anchor, byteFault: "valid", gasMode: "normal" })
  }

  // Zero cannot execute; an infinite child cannot finish with finite gas.
  // The funded control executes the same declarations with the same gas budget.
  run gas_exhaustion_works = replayInit
    .then(invoke({ ...input(declarations), gasMode: "zero" }))
    .then(invoke({ ...input(declarations), gasMode: "funded" }))
    .then(invoke({ ...input(declarations), gasMode: "child" }))
    .then(invoke({ ...input(declarations), gasMode: "child", available: false }))
    .then(invoke({ ...input(declarations), gasMode: "funded" }))

  run malformed_candidate_works = replayInit
    .then(invoke({ ...input(declarations), byteFault: "candidate_empty" }))
    .then(invoke({ ...input(declarations), byteFault: "candidate_truncated" }))
    .then(invoke({ ...input(declarations), byteFault: "candidate_trailing" }))
    .then(invoke({ ...input(declarations), byteFault: "candidate_empty", available: false }))
    .then(invoke(input(declarations)))
  run invalid_pvf_works = replayInit
    .then(invoke({ ...input(declarations), byteFault: "pvf_empty" }))
    .then(invoke({ ...input(declarations), byteFault: "pvf_magic" }))
    .then(invoke({ ...input(declarations), byteFault: "pvf_truncated" }))
    .then(invoke({ ...input(declarations), byteFault: "pvf_magic", available: false }))
    .then(invoke(input(declarations)))
  run structured_pvf_works = replayInit
    .then(invoke({ ...input(declarations), byteFault: "pvf_memory" }))
    .then(invoke({ ...input(declarations), byteFault: "pvf_entry" }))
    .then(invoke({ ...input(declarations), byteFault: "pvf_memory", available: false }))
    .then(invoke({ ...input(declarations), byteFault: "pvf_entry", available: false }))
    .then(invoke(input(declarations)))
  run success_works = replayInit.then(invoke(input(declarations)))
  run ordering_works = replayInit
    .then(invoke(input(List(ReportError(64), Panic))))
    .then(invoke(input(List(Panic, ReportError(64)))))
    .then(invoke(input(List(SetHead(1), SetHeadOversized))))
    .then(invoke(input(List(SetHeadOversized, SetHead(1)))))
  run lookup_works = replayInit.then(invoke({ ...input(List(Panic)), available: false }))
}
