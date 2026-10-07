// GP 0.7.2 compatibility for service management. Keep the vendored design pin intact.
// The mock is registrar. Its allocator restarts at 65536 each invocation and
// advances by 42 after public allocations; protected allocations do not advance it.
module service_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import foreign_services.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/foreign_services"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  import preimage_history.* from "./preimage_history"

  type CreationEffect = { service: ServiceId, args: CreateServiceArgs }
  var replayCreations: List[CreationEffect]
  action noCreations = replayCreations' = List()

  pure def publicId(fs: ForeignServices, slot: int): ServiceId = {
    val start = fs.keys().fold(MinPublicServiceIndex, (n, id) => match id {
      | MkServiceId(i) => if (i >= MinPublicServiceIndex and fs.get(id).created == slot and i + 42 > n) i + 42 else n
      | _ => n
    })
    MkServiceId(start.to(start + fs.keys().size()).filter(i => not(fs.keys().contains(MkServiceId(i))))
      .fold(start + fs.keys().size(), (n, i) => if (i < n) i else n))
  }
  pure def createArgs(desired: Option[ServiceId], id: int): CreateServiceArgs = {
    codeHash: { hashBytes: 8801 }, len: 1024, minItemGas: 123, minMemoGas: 200,
    id: id, desiredId: desired, sourceSupervisorBalance: false, newSupervisorBalance: false
  }
  pure def serviceMessages(fs: ForeignServices): Set[UpwardMessage] = {
    val targets = fs.keys().union(Set(ParachainServiceId, MkServiceId(99)))
    targets.map(id => EjectService({ service: id })).union(
      targets.fold(Set(), (msgs, id) => msgs.union(Set(serviceSolicit(id), serviceForget(id), serviceRemove(id))
        .union(targets.map(u => serviceSupervisor(id, u))))))
    .union(Set(
      CreateService(createArgs(None, 77)),
      CreateService(createArgs(Some(MkServiceId(42)), 77)),
      CreateService(createArgs(Some(MkServiceId(70000)), 78)),
      CreateService({ ...createArgs(None, 79), sourceSupervisorBalance: true }),
      CreateService({ ...createArgs(None, 80), newSupervisorBalance: true })
    ))
  }
  // Use the creation code request as well as absent requests on seeded accounts.
  pure def serviceSolicit(id: ServiceId): UpwardMessage =
    Solicit({ target: Service(id), hash: { hashBytes: 8801 }, len: 1024 })
  pure def serviceForget(id: ServiceId): UpwardMessage =
    Forget({ target: Service(id), hash: { hashBytes: 8801 }, len: 1024 })
  pure def serviceRemove(id: ServiceId): UpwardMessage =
    RemoveServiceStorage({ service: id, key: List(0, 255, 1) })
  pure def serviceSupervisor(id: ServiceId, successor: ServiceId): UpwardMessage =
    SetServiceSupervisor({ service: id, newSupervisor: successor })
  pure def replayApplyBase(acc: AccumulateOutcome, caller: ParaId, codeLen: int,
    slot: int, anchor: int, msg: UpwardMessage): AccumulateOutcome = match msg {
    // The pin looks only in foreignServices, which excludes this service.
    // The host knows self exists, but still exposes no supervision capability.
    | Solicit(p) => if (p.target == Service(ParachainServiceId))
        { ...acc, logs: acc.logs.append(ServiceSolicitFailed({ service: ParachainServiceId, error: SolicitNotSupervised })) }
      else applyUpwardMessage(acc, caller, codeLen, slot, anchor, msg)
    | Forget(p) => if (p.target == Service(ParachainServiceId))
        { ...acc, logs: acc.logs.append(ServiceStoreFailed({ service: ParachainServiceId, error: StoreNotSupervised })) }
      else applyUpwardMessage(acc, caller, codeLen, slot, anchor, msg)
    | RemoveServiceStorage(p) => if (p.service == ParachainServiceId)
        { ...acc, logs: acc.logs.append(ServiceStoreFailed({ service: p.service, error: StoreNotSupervised })) }
      else applyUpwardMessage(acc, caller, codeLen, slot, anchor, msg)
    | SetServiceSupervisor(p) => if (p.service == ParachainServiceId)
        { ...acc, logs: acc.logs.append(ServiceSupervisorFailed({ service: p.service,
          error: if (p.newSupervisor == ParachainServiceId or acc.foreign.keys().contains(p.newSupervisor))
            HandoffNotSupervised else HandoffUnknownNewSupervisor })) }
      else applyUpwardMessage(acc, caller, codeLen, slot, anchor, msg)
    | CreateService(c) => {
      if (c.sourceSupervisorBalance or c.newSupervisorBalance)
        { ...acc, logs: acc.logs.append(ServiceCreation({ id: c.id, result: CannotAfford })) }
      else {
        val fresh = match c.desiredId {
          | Some(id) => match id { | MkServiceId(i) => if (i < MinPublicServiceIndex) id else publicId(acc.foreign, slot) | _ => publicId(acc.foreign, slot) }
          | None => publicId(acc.foreign, slot)
        }
        val out = foreignCreate(acc.state, acc.foreign, { ...c, desiredId: Some(fresh) }, slot)
        // The host records parentage, but provides no supervision capabilities.
        val fs = if (out.services.keys().contains(fresh) and not(acc.foreign.keys().contains(fresh)))
          out.services.put(fresh, { ...out.services.get(fresh), supervisor: fresh, supervisorCreated: slot })
          else out.services
        // Pinned host debits before testing a protected ID collision.
        val state = if (out.log == Some(ServiceCreation({ id: c.id, result: IdTaken })))
          { ...out.state, jamAccount: out.state.jamAccount.debit(BaseDeposit + 2 * ItemDeposit + PreimageRequestOverhead + c.len, false) }
          else out.state
        { ...acc, state: state, foreign: fs, logs: acc.logs.appendOption(out.log) }
      }
    }
    | _ => applyUpwardMessage(acc, caller, codeLen, slot, anchor, msg)
  }

  pure def replayApply(acc: AccumulateOutcome, caller: ParaId, codeLen: int,
    slot: int, anchor: int, msg: UpwardMessage): AccumulateOutcome = {
    val out = replayApplyBase(acc, caller, codeLen, slot, anchor, msg)
    { ...out, preimageOps: out.preimageOps.concat(releasedReferences(acc.state, out.state)) }
  }

  pure def replayPackage(state: ParachainServiceState, wr: WorkResult,
    solicited: Set[(ParaId, Hash, int)], designate: List[ValidatorKey], codeLen: int,
    slot: int, foreign: ForeignServices): PerPackageOutcome = {
    val plain = accumulateOnePackage(state, wr, solicited, designate, codeLen, slot, foreign)
    match wr.result {
      | WorkOk(digest) => match digest {
      | Ok(ok) => {
        val pre = accumulateOkPrefix(state, ok, codeLen, slot)
        if (pre.rejected) plain else {
          val base: AccumulateOutcome = { state: pre.state, foreign: foreign,
            jamDesignate: designate, preimageOps: releasedReferences(state, pre.state),
            jamAssigns: List(), logs: pre.logs }
          val out = ok.upwardMessages.foldl(base, (a, m) => replayApply(a, ok.paraId, codeLen, slot, ok.lookupAnchor, m))
          { ...plain, state: out.state.appendAccumulateLogs(ok.paraId, slot, out.logs),
            foreign: out.foreign, solicited: applyPreimageOps(solicited, out.preimageOps),
            jamDesignate: out.jamDesignate, jamAssigns: out.jamAssigns }
        }
      }
      | _ => plain
      }
      | _ => plain
    }
  }
  pure def replayAll(state: ParachainServiceState, solicited: Set[(ParaId, Hash, int)],
    designate: List[ValidatorKey], codeLen: int, slot: int, packages: List[WorkResult],
    foreign: ForeignServices): MultiPackageOutcome =
    packages.foldl({ state: state, solicited: solicited, jamDesignate: designate,
      jamAssigns: List(), logPruned: List(), foreign: foreign }, (a, wr) => {
      val out = replayPackage(a.state, wr, a.solicited, a.jamDesignate, codeLen, slot, a.foreign)
      { state: out.state, solicited: out.solicited, jamDesignate: out.jamDesignate,
        jamAssigns: a.jamAssigns.concat(out.jamAssigns), foreign: out.foreign,
        logPruned: a.logPruned.appendOption(out.logPruned) }
    })
}
