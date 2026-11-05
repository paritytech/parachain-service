module fuzz {
  import code_storage_inputs.* from "./code_storage_inputs"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  action replayInit = codeInit
  action replayStep = any { codeRecoveryStep, codeProvision, announcementStep }
  // Keep the solicitation -> provision -> announcement chain reachable even
  // when random invocations repeatedly roll back their acquisition writes.
  action announcementStep = {
    val hash = 777
    val key = ({ hashBytes: hash }, FixedCodeLen)
    if (not(svc.preimageStatus.keys().contains(key)))
      codeBlock(1000000, Solicit({ target: Parachain(CoretimeParaId),
        hash: { hashBytes: hash }, len: FixedCodeLen }))
    else if (svc.preimageStatus.get(key) == Unprovided) codeProvide(hash)
    else {
      nondet allowance = Set(0, 35, 36).oneOf()
      nondet stop = Set(-2, -1, 0, 1).oneOf()
      codeRecoveryBlock(allowance, announce(hash), stop)
    }
  }
  run acquisition_recovery_works = codeInit
    .then(codeBlock(1000000, ParachainSetStateBalance({ paraId: MkParaId(3), newTotal: 1073741823 })))
    .then(codeRecoveryBlock(85, forcedCode(MkParaId(3), 777), -2))
    .then(codeRecoveryBlock(85, forcedCode(MkParaId(3), 777), 0))
    .then(codeRecoveryBlock(85, forcedCode(MkParaId(3), 777), 1))
    .then(codeBlock(86, forcedCode(MkParaId(3), 777)))
    .then(codeRecoveryBlock(65723, forcedCode(MkParaId(3), 777), 0))
    .then(codeRecoveryBlock(65723, forcedCode(MkParaId(3), 777), 1))
    .then(codeRecoveryBlock(35, forcedCode(MkParaId(3), 777), 1))
    .then(codeRecoveryBlock(36, forcedCode(MkParaId(3), 777), 0))
    .then(codeBlock(36, forcedCode(MkParaId(3), 777)))
  run announcement_recovery_works = codeInit
    .then(codeBlock(1000000, Solicit({ target: Parachain(CoretimeParaId), hash: { hashBytes: 777 }, len: FixedCodeLen })))
    .then(codeProvide(777))
    .then(codeRecoveryBlock(35, announce(777), -2))
    .then(codeRecoveryBlock(35, announce(777), 0))
    .then(codeRecoveryBlock(35, announce(777), 1))
    .then(codeRecoveryBlock(36, announce(777), 0))
    .then(codeRecoveryBlock(36, announce(777), 1))
    .then(codeBlock(0, announce(777)))
}
