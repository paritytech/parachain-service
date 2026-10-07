// Independent host-write oracle for code acquisition and announcements.
module code_storage_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"
  import management.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/management"
  import code_upgrades.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/code_upgrades"
  import head_commitment.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/head_commitment"
  import host_sizes.* from "./host_sizes"
  import host_metadata.* from "./host_metadata"
  import gas_inputs.* from "./gas_inputs"
  import transfer_inputs.* from "./transfer_inputs"
  import validator_key_inputs.* from "./validator_key_inputs"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  var replayHostTrap: bool
  // Diagnostic coverage label, calculated from footprints before execution.
  var replayCodeFailure: str
  type CodeHost = { state: ParachainServiceState, free: int,
    logs: List[AccumulateLog], failure: str, trap: bool }
  pure def codeReject(h: CodeHost, label: str): CodeHost = { ...h, failure: label }
  pure def codeWrite(h: CodeHost, proposed: ParachainServiceState, label: str): CodeHost = {
    val delta = hostCost(proposed) - hostCost(h.state)
    if (delta > h.free) codeReject(h, label)
    else { ...h, state: proposed, free: h.free - delta }
  }
  pure def acquire(h: CodeHost, para: ParaId, hash: Hash, len: int): CodeHost = {
    val out = h.state.addReferencer(para, hash, len)
    val charged = { ...h.state, parachains: out.state.parachains }
    val registered = { ...charged, preimageRegistry: out.state.preimageRegistry }
    val chargeCost = hostCost(charged) - hostCost(h.state)
    val registryCost = hostCost(registered) - hostCost(h.state)
    val totalCost = hostCost(out.state) - hostCost(h.state)
    if (out.log != None) { ...h, logs: match out.log { | Some(log) => h.logs.append(log) | None => h.logs } }
    else if (chargeCost > h.free or registryCost > h.free) {
      ...h, failure: if (chargeCost > h.free) "charge" else "registry",
      logs: h.logs.append(InsufficientStateBalance(FromSolicit({ hash: hash, len: len })))
    } else if (totalCost > h.free) { ...h, trap: true, failure: "solicit" }
    else { ...h, state: out.state, free: h.free - totalCost }
  }
  pure def codeMessage(h: CodeHost, msg: UpwardMessage): CodeHost = match msg {
    | ParachainSetStateBalance(p) => codeWrite(h,
        parachainSetStateBalance(h.state, p.paraId, p.newTotal).state, "registration")
    | ParachainSetValidationCode(p) => if (not(h.state.isLive(p.paraId))) h else {
      val a = acquire(h, p.paraId, p.newValidationCode.hash.asPreimage(), p.newValidationCode.len)
      if (a.trap or a.logs != h.logs) a else codeWrite(a,
        parachainSetValidationCode(a.state, p.paraId, p.newValidationCode.hash,
          p.newValidationCode.len, 0).state, "forced-metadata")
    }
    | Solicit(p) => acquire(h, CoretimeParaId, p.hash, p.len)
    | RequestCodeUpgrade(p) => {
      val out = announceCodeUpgrade(h.state, CoretimeParaId, p.hash, p.len, 0)
      match out.log {
        | Some(log) => { ...h, logs: h.logs.append(log) }
        | None => codeWrite(h, out.state, "announcement")
      }
    }
    | _ => h
  }
  pure def forcedCode(target: ParaId, hash: int): UpwardMessage =
    ParachainSetValidationCode({ paraId: target,
      newValidationCode: { hash: { vchBytes: hash }, len: FixedCodeLen } })
  pure def announce(hash: int): UpwardMessage = RequestCodeUpgrade({
    hash: { vchBytes: hash }, len: FixedCodeLen, phase: Announcement })
  action codeInit = all { init, noTransfers, noDesignation, noGas,
    replayHostTrap' = false, replayCodeFailure' = "" }
  action codeBlock(allowance: int, msg: UpwardMessage): bool = {
    val start = { state: svc, free: allowance, logs: List(), failure: "", trap: false }
    val applied = codeMessage(start, msg)
    val logged = applied.state.appendAccumulateLogs(CoretimeParaId, now + 1, applied.logs)
    val saved = codeWrite(applied, logged, applied.failure)
    val out = if (applied.trap) start else saved
    val pi = svc.paraInfo(CoretimeParaId)
    val wr = { result: WorkOk(Ok({ paraId: CoretimeParaId,
      validationCode: activeCodeHashOr(pi, { vchBytes: 0 }),
      parentHeadHash: headHash(pi.headData), headData: pi.headData,
      lookupAnchor: 0, upwardMessages: List(msg) })), authTrace: 0 }
    all {
      svc' = out.state, prevSvc' = svc, now' = now + 1,
      lastStepWorkResults' = List(wr), lastStepAssigns' = List(),
      lastHeadRoot' = if (applied.trap) None else headCommitment(svc, out.state),
      solicitedSet' = solicitedSet, jamStagingSet' = jamStagingSet,
      foreignServices' = foreignServices, logPrunedBelow' = logPrunedBelow,
      noTransfers, noDesignation,
      replayHostBudget' = allowance, replayHostFree' = out.free,
      replayHostRejects' = if (applied.failure == "") 0 else 1,
      replayGasLimits' = List(UnlimitedGas), replayInterrupt' = if (applied.trap) 0 else -1,
      replayPanic' = false, replayHostTrap' = applied.trap,
      replayCodeFailure' = applied.failure
    }
  }
  action codeProvide(hash: int): bool = all {
    svc.preimageStatus.keys().contains(({ hashBytes: hash }, FixedCodeLen)),
    svc.preimageStatus.get(({ hashBytes: hash }, FixedCodeLen)) == Unprovided,
    provide({ hashBytes: hash }, FixedCodeLen), noTransfers, noDesignation, noGas,
    replayHostTrap' = false, replayCodeFailure' = ""
  }
  action codeStep = {
    nondet allowance = oneOf(Set(0, 1, 2, 35, 36, 85, 86, 87, 88, 65722, 65723, 65759, 1000000))
    nondet hash = oneOf(Set(777, 778, 779))
    nondet target = oneOf(Set(MkParaId(3), MkParaId(4)))
    nondet msg = oneOf(Set(
      ParachainSetStateBalance({ paraId: target, newTotal: 1073741823 }),
      ParachainSetStateBalance({ paraId: CoretimeParaId, newTotal: 1073741823 }),
      forcedCode(target, hash),
      Solicit({ target: Parachain(CoretimeParaId), hash: { hashBytes: hash }, len: FixedCodeLen }),
      announce(hash)))
    codeBlock(allowance, msg)
  }
  action codeProvision = { nondet hash = oneOf(Set(777, 778, 779)); codeProvide(hash) }
}
