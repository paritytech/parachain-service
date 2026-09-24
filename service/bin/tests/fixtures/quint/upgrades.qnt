// Code-upgrade lifecycle sequences evaluated by the pinned Quint model.
//
// The model's `announce`/`apply` lifecycle (§5.2) has no deadline and no reap:
// a `RequestCodeUpgrade({ phase: Announcement })` makes the announced code
// validation code until an `Apply` (or a superseding announcement) replaces it.
// These scenarios mirror
// `tests/code_upgrades_test.qnt` through the block-level helpers so the traces
// stay replayable.
module upgrades {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"

  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"

  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"

  // CoretimeParaId's active code at `init`.
  pure val codeA: ValidationCodeHash = { vchBytes: 1 }
  pure val codeB: ValidationCodeHash = { vchBytes: 777 }
  pure val codeC: ValidationCodeHash = { vchBytes: 778 }

  pure def announcedHashOf(pi: ParaInfo): Option[ValidationCodeHash] =
    match pi.announcedUpgrade { | None => None | Some(vc) => Some(vc.hash) }

  pure def hasLog(
    s: ParachainServiceState, para: ParaId, entry: AccumulateLog
  ): bool =
    s.paraLog(para).foldl(false, (found, e) => found or match e._2 {
      | AccumulateLogEntry(events) =>
        events.foldl(false, (seen, event) => seen or event == entry)
      | _ => false
    })

  // Raise Coretime's total so `count` further FixedCodeLen preimages can be
  // solicited on top of the single extra slot `InitialParaBalance` leaves.
  action fundForExtraCodes(count: int): bool =
    applyHappyPath(CoretimeParaId, codeA, List(
      ParachainSetStateBalance({ paraId: CoretimeParaId,
        newTotal: svc.paraInfo(CoretimeParaId).usedStateBalance
          + preimageFootprint(FixedCodeLen) * count })))

  // 1. The active code is supplied.
  // 2. The para solicits the new code itself; someone supplies the blob.
  // 3. The para announces the new code; the active code is untouched.
  // 4. Blocks pass with no deadline: the announcement still stands.
  // 5. The para applies the new code: `validationCode` flips, `announcedUpgrade`
  //    clears, and the displaced code stays referenced and charged until the
  //    para forgets it.
  run activation_works = {
    val b = codeB
    val oldKey = (codeA.asPreimage(), FixedCodeLen)
    init
      .then(provide(codeA.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      .then(expectThat(all {
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b),
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: codeA, len: FixedCodeLen }),
        svc.preimageStatus.get((b.asPreimage(), FixedCodeLen)) == Provided
      }))
      // No deadline: an announcement outlives any number of empty blocks.
      .then(applyHappyPath(CoretimeParaId, codeA, List()))
      .then(applyHappyPath(CoretimeParaId, codeA, List()))
      .then(expectThat(announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b)))
      // Apply. The candidate carrying it is still validated with the old code.
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Apply }))))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: b, len: FixedCodeLen }),
        svc.paraInfo(CoretimeParaId).announcedUpgrade == None,
        svc.hasReferencer(b.asPreimage(), FixedCodeLen, CoretimeParaId),
        // The displaced code is no longer validation code: still referenced and charged.
        not(isValidationCodeOf(svc.paraInfo(CoretimeParaId), codeA.asPreimage(), FixedCodeLen)),
        svc.hasReferencer(codeA.asPreimage(), FixedCodeLen, CoretimeParaId)
      }))
      // Only now can the para reclaim the old code; it is available, so the
      // first forget only unrequests it.
      .then(applyHappyPath(CoretimeParaId, b, List(
        Forget({ target: Parachain(CoretimeParaId), hash: codeA.asPreimage(), len: FixedCodeLen }))))
      .then(expectThat(isUnrequested(svc.preimageStatus.get(oldKey))))
  }

  // Apply only swaps the two code slots: the displaced, still-provided code
  // keeps its referencer, its charge and its `Provided` status, and is released
  // only by the para's own later forget (which is two-step).
  run provided_old_code_activation_works = {
    val b = codeB
    val oldKey = (codeA.asPreimage(), FixedCodeLen)
    init
      .then(provide(codeA.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Apply }))))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: b, len: FixedCodeLen }),
        svc.paraInfo(CoretimeParaId).announcedUpgrade == None,
        svc.hasReferencer(codeA.asPreimage(), FixedCodeLen, CoretimeParaId),
        svc.preimageStatus.get(oldKey) == Provided,
        not(isValidationCodeOf(svc.paraInfo(CoretimeParaId), codeA.asPreimage(), FixedCodeLen)),
        svc.paraInfo(CoretimeParaId).usedStateBalance ==
          prevSvc.paraInfo(CoretimeParaId).usedStateBalance
      }))
      .then(applyHappyPath(CoretimeParaId, b, List(
        Forget({ target: Parachain(CoretimeParaId), hash: codeA.asPreimage(), len: FixedCodeLen }))))
      .then(expectThat(all {
        isUnrequested(svc.preimageStatus.get(oldKey)),
        hasLog(svc, CoretimeParaId, ForgetAgainAt({
          hash: codeA.asPreimage(), len: FixedCodeLen, due: now + ExpungePeriod }))
      }))
  }

  // Announcing a code that was solicited but never supplied is rejected: nothing
  // is announced and the failure is logged.
  run announcement_unavailable_rejected_works = {
    val b = codeB
    init
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).announcedUpgrade == None,
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: codeA, len: FixedCodeLen }),
        svc.preimageStatus.get((b.asPreimage(), FixedCodeLen)) == Unprovided,
        hasLog(svc, CoretimeParaId,
          CodeUpgradeNotAvailable({ hash: b.asPreimage(), len: FixedCodeLen }))
      }))
  }

  // An available code the parachain does NOT reference cannot be announced:
  // otherwise a para could announce a code another para paid for.
  run announcement_of_other_paras_code_rejected_works = {
    val b = codeB
    init
      .then(applyHappyPath(AssetHubParaId, { vchBytes: 2 }, List(
        Solicit({ target: Parachain(AssetHubParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      .then(expectThat(all {
        svc.preimageAvailable(b.asPreimage(), FixedCodeLen),
        not(svc.hasReferencer(b.asPreimage(), FixedCodeLen, CoretimeParaId)),
        svc.paraInfo(CoretimeParaId).announcedUpgrade == None,
        hasLog(svc, CoretimeParaId,
          CodeUpgradeNotAvailable({ hash: b.asPreimage(), len: FixedCodeLen }))
      }))
  }

  // `Apply` with nothing announced is rejected; the active code is unchanged.
  run apply_without_announcement_rejected_works = {
    val b = codeB
    init
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Apply }))))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: codeA, len: FixedCodeLen }),
        svc.paraInfo(CoretimeParaId).announcedUpgrade == None,
        hasLog(svc, CoretimeParaId,
          CodeUpgradeNotAnnounced({ hash: b.asPreimage(), len: FixedCodeLen }))
      }))
  }

  // `Apply` naming a code different from the standing announcement is rejected;
  // the announcement is preserved.
  run apply_mismatched_announcement_rejected_works = {
    val b = codeB
    val c = codeC
    init
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: c, len: FixedCodeLen, phase: Apply }))))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: codeA, len: FixedCodeLen }),
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b),
        hasLog(svc, CoretimeParaId,
          CodeUpgradeNotAnnounced({ hash: c.asPreimage(), len: FixedCodeLen }))
      }))
  }

  // A second announcement supersedes the first. The superseded code releases
  // nothing: its referencer is the para's own solicit and stands until forgotten.
  run announcement_supersedes_previous_works = {
    val b = codeB
    val c = codeC
    init
      .then(fundForExtraCodes(2))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: c.asPreimage(), len: FixedCodeLen }))))
      .then(provide(c.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      .then(expectThat(announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b)))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: c, len: FixedCodeLen, phase: Announcement }))))
      .then(expectThat(all {
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(c),
        not(isValidationCodeOf(svc.paraInfo(CoretimeParaId), b.asPreimage(), FixedCodeLen)),
        svc.hasReferencer(b.asPreimage(), FixedCodeLen, CoretimeParaId),
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: codeA, len: FixedCodeLen })
      }))
  }

  // A `Forget` of the announced code is refused while it is the announced code,
  // and goes through once a superseding announcement has displaced it.
  run forget_announced_code_refused_works = {
    val b = codeB
    val c = codeC
    init
      .then(fundForExtraCodes(2))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: c.asPreimage(), len: FixedCodeLen }))))
      .then(provide(c.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      // Refused while announced: the reference, charge and status all stay.
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Forget({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(expectThat(all {
        svc.preimageStatus.get((b.asPreimage(), FixedCodeLen)) == Provided,
        svc.hasReferencer(b.asPreimage(), FixedCodeLen, CoretimeParaId),
        isValidationCodeOf(svc.paraInfo(CoretimeParaId), b.asPreimage(), FixedCodeLen),
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b),
        hasLog(svc, CoretimeParaId,
          CanNotForgetValidationCode({ hash: b.asPreimage(), len: FixedCodeLen }))
      }))
      // Supersede with C, which displaces B; only then does the forget take effect.
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: c, len: FixedCodeLen, phase: Announcement }))))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Forget({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(expectThat(all {
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(c),
        isUnrequested(svc.preimageStatus.get((b.asPreimage(), FixedCodeLen))),
        hasLog(svc, CoretimeParaId, ForgetAgainAt({
          hash: b.asPreimage(), len: FixedCodeLen, due: now + ExpungePeriod }))
      }))
  }

  // A second request rejected for insufficient balance leaves the standing
  // announcement intact, and the unaffordable code then fails to announce.
  run insufficient_balance_preserves_announcement_works = {
    val b = codeB
    val c = codeC
    init
      // Headroom for exactly one more preimage.
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        ParachainSetStateBalance({ paraId: CoretimeParaId,
          newTotal: svc.paraInfo(CoretimeParaId).usedStateBalance
            + preimageFootprint(FixedCodeLen) }))))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      // The second solicit cannot be afforded: rejected and logged.
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: c.asPreimage(), len: FixedCodeLen }))))
      .then(expectThat(all {
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b),
        not(svc.preimageRegistry.keys().contains((c.asPreimage(), FixedCodeLen))),
        hasLog(svc, CoretimeParaId,
          InsufficientStateBalance(FromSolicit({ hash: c.asPreimage(), len: FixedCodeLen })))
      }))
      // Announcing the unaffordable code is then rejected as unavailable.
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: c, len: FixedCodeLen, phase: Announcement }))))
      .then(expectThat(all {
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b),
        hasLog(svc, CoretimeParaId,
          CodeUpgradeNotAvailable({ hash: c.asPreimage(), len: FixedCodeLen }))
      }))
  }

  // Jump to a later block slot without manufacturing intermediate state.
  action workErrorAt(slot: Timeslot): bool = {
    val out = accumulateBlock(
      svc, slot, jamStagingSet, solicitedSet, FixedCodeLen,
      List({ result: WorkErr, authTrace: 7 }), foreignServices)
    all {
      slot > now,
      svc' = out.state,
      foreignServices' = out.foreign,
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

  // Skipped work preserves the announcement (there is no deadline), and a later
  // `Apply` still lands, validated by the old active code.
  run work_error_preserves_announcement_works = {
    val b = codeB
    init
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        Solicit({ target: Parachain(CoretimeParaId), hash: b.asPreimage(), len: FixedCodeLen }))))
      .then(provide(b.asPreimage(), FixedCodeLen))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Announcement }))))
      .then(workErrorAt(now + MaxLookupAge))
      .then(expectThat(all {
        svc == prevSvc,
        announcedHashOf(svc.paraInfo(CoretimeParaId)) == Some(b)
      }))
      .then(applyHappyPath(CoretimeParaId, codeA, List(
        RequestCodeUpgrade({ hash: b, len: FixedCodeLen, phase: Apply }))))
      .then(expectThat(all {
        svc.paraInfo(CoretimeParaId).validationCode == Some({ hash: b, len: FixedCodeLen }),
        svc.paraInfo(CoretimeParaId).announcedUpgrade == None
      }))
  }
}
