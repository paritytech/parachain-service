// Upgrade lifecycle sequences evaluated by the pinned Quint model.
module upgrades {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"

  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"

  import tests.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests"

  run insufficient_balance_preserves_pending_works = {
    val first = { vchBytes: 777 }
    val second = { vchBytes: 778 }
    init
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(
        ParachainSetStateBalance({ paraId: CoretimeParaId,
          newTotal: svc.paraInfo(CoretimeParaId).usedStateBalance + preimageFootprint(FixedCodeLen) }))))
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(
        RequestCodeUpgrade({ hash: first, len: FixedCodeLen }),
        RequestCodeUpgrade({ hash: second, len: FixedCodeLen }))))
      .then(expectThat(all {
        match svc.paraInfo(CoretimeParaId).pendingUpgrade {
          | None => false
          | Some(pair) => pair._1.ref.hash == first
        },
        not(svc.preimageRegistry.keys().contains((vchAsHash(second), FixedCodeLen))),
        svc.paraLog(CoretimeParaId).foldl(false, (found, entry) => found or match entry._2 {
          | AccumulateLogEntry(events) => events.foldl(false, (foundEvent, event) =>
              foundEvent or event == InsufficientStateBalance(
                FromSolicit({ hash: vchAsHash(second), len: FixedCodeLen })))
          | _ => false
        })
      }))
  }

  // Known model/Rust differences exposed by upgrade fuzzing. These runs pin
  // the model's behavior; Rust tests below assert the specific replay mismatch.
  action providedUpgrade =
    applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List(
      RequestCodeUpgrade({ hash: { vchBytes: 777 }, len: FixedCodeLen })))

  run provided_upgrade_expiry_log_works =
    init.then(providedUpgrade)
      .then(applyHappyPaths(List()))
      .then(provide({ hashBytes: 777 }, FixedCodeLen))
      .then(workErrorAt(UpgradeTimeoutTimeslots + 2))
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 1 }, List()))
      .then(expectThat(svc.paraInfo(CoretimeParaId).pendingUpgrade == None))

  run expired_code_candidate_reaps_works =
    init.then(providedUpgrade)
      .then(applyHappyPaths(List()))
      .then(provide({ hashBytes: 777 }, FixedCodeLen))
      .then(workErrorAt(UpgradeTimeoutTimeslots + 2))
      .then(applyHappyPath(CoretimeParaId, { vchBytes: 777 }, List()))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).pendingUpgrade == None,
        svc.paraInfo(CoretimeParaId).headData == 1
      }))

  run activation_works = {
    val oldCode: ValidationCodeHash = { vchBytes: 1 }
    val newCode: ValidationCodeHash = { vchBytes: 778 }
    val oldKey = (vchAsHash(oldCode), FixedCodeLen)
    val newKey = (vchAsHash(newCode), FixedCodeLen)
    init
      .then(applyHappyPath(CoretimeParaId, oldCode, List(
        RequestCodeUpgrade({ hash: newCode, len: FixedCodeLen }))))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).validationCode ==
          initialState.paraInfo(CoretimeParaId).validationCode,
        svc.paraInfo(CoretimeParaId).pendingUpgrade == Some((
          { ref: { hash: newCode, len: FixedCodeLen }, pinned: false },
          now + UpgradeTimeoutTimeslots)),
        svc.preimageStatus.get(newKey) == Unprovided,
        svc.hasReferencer(vchAsHash(newCode), FixedCodeLen, CoretimeParaId),
        svc.paraInfo(CoretimeParaId).usedStateBalance >
          prevSvc.paraInfo(CoretimeParaId).usedStateBalance
      }))
      // Clear per-block work results before external preimage provision.
      .then(applyHappyPaths(List()))
      .then(provide(vchAsHash(newCode), FixedCodeLen))
      .then(expectThat(all {
        svc.preimageStatus.get(newKey) == Provided,
        svc.paraInfo(CoretimeParaId) == prevSvc.paraInfo(CoretimeParaId)
      }))
      // Provision alone does not activate; old-code candidates remain valid.
      .then(applyHappyPath(CoretimeParaId, oldCode, List()))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).headData == 2,
        svc.paraInfo(CoretimeParaId).pendingUpgrade ==
          prevSvc.paraInfo(CoretimeParaId).pendingUpgrade,
        svc.paraInfo(CoretimeParaId).validationCode ==
          initialState.paraInfo(CoretimeParaId).validationCode
      }))
      .then(applyHappyPath(CoretimeParaId, newCode, List()))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).headData == 3,
        svc.paraInfo(CoretimeParaId).pendingUpgrade == None,
        svc.paraInfo(CoretimeParaId).validationCode == Some(
          { ref: { hash: newCode, len: FixedCodeLen }, pinned: false }),
        svc.preimageStatus.get(newKey) == Provided,
        not(svc.preimageStatus.keys().contains(oldKey)),
        not(svc.preimageRegistry.keys().contains(oldKey)),
        svc.hasReferencer(vchAsHash(newCode), FixedCodeLen, CoretimeParaId),
        svc.paraInfo(CoretimeParaId).usedStateBalance ==
          initialState.paraInfo(CoretimeParaId).usedStateBalance,
        svc.paraInfo(CoretimeParaId).totalStateBalance ==
          initialState.paraInfo(CoretimeParaId).totalStateBalance,
        svc.parachainLog == prevSvc.parachainLog
      }))
  }

  // Jump to a later block slot without manufacturing intermediate state.
  action workErrorAt(slot: Timeslot): bool = {
    val out = accumulateBlock(
      svc, slot, jamStagingSet, solicitedSet, FixedCodeLen,
      List({ result: WorkErr, authTrace: 7 }))
    all {
      slot > now,
      svc' = out.state,
      now' = slot,
      jamStagingSet' = out.staging,
      solicitedSet' = out.solicited,
      prevSvc' = svc,
      lastStepWorkResults' = out.results,
      lastStepAssigns' = out.assigns,
      lastHeadRoot' = out.headRoot,
      logPrunedBelow' = logPrunedBelow
    }
  }

  run work_error_across_upgrade_deadline_works = {
    val oldCode: ValidationCodeHash = { vchBytes: 1 }
    val newCode: ValidationCodeHash = { vchBytes: 777 }
    val key = (vchAsHash(newCode), FixedCodeLen)
    init
      .then(applyHappyPath(CoretimeParaId, oldCode, List(
        RequestCodeUpgrade({ hash: newCode, len: FixedCodeLen })
      )))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).headData == 1,
        match svc.paraInfo(CoretimeParaId).pendingUpgrade {
          | None => false
          | Some(pair) => all {
              pair._1.ref.hash == newCode,
              not(pair._1.pinned),
              pair._2 == now + UpgradeTimeoutTimeslots
            }
        },
        svc.hasReferencer(vchAsHash(newCode), FixedCodeLen, CoretimeParaId),
        svc.preimageStatus.get(key) == Unprovided,
        svc.paraInfo(CoretimeParaId).usedStateBalance >
          prevSvc.paraInfo(CoretimeParaId).usedStateBalance
      }))
      // The request was at slot 1; this block is one slot past its deadline.
      .then(workErrorAt(UpgradeTimeoutTimeslots + 2))
      .then(expectThat(all {
        svc == prevSvc,
        match svc.paraInfo(CoretimeParaId).pendingUpgrade {
          | None => false
          | Some(pair) => pair._2 < now
        }
      }))
      .then(applyHappyPath(CoretimeParaId, oldCode, List()))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).headData == 2,
        svc.paraInfo(CoretimeParaId).pendingUpgrade == None,
        svc.paraInfo(CoretimeParaId).validationCode ==
          initialState.paraInfo(CoretimeParaId).validationCode,
        svc.paraInfo(CoretimeParaId).usedStateBalance ==
          initialState.paraInfo(CoretimeParaId).usedStateBalance,
        svc.paraInfo(CoretimeParaId).totalStateBalance ==
          initialState.paraInfo(CoretimeParaId).totalStateBalance,
        not(svc.preimageRegistry.keys().contains(key)),
        not(svc.preimageStatus.keys().contains(key))
      }))
  }
}
