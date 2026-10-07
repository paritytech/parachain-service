// Storage-backstop inputs with no allowance refusals: baseline reservations are
// funded, while actual JAM free balance varies independently.
module storage_metadata_inputs {
  import host_sizes.* from "./host_sizes"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import management.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/management"
  import storage_inputs.* from "./storage_inputs"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import state_balance.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_balance"

  def metadataWork(caller: ParaId, msgs: List[UpwardMessage]): WorkResult = {
    result: WorkOk(Ok({ paraId: caller,
      validationCode: activeCodeHashOr(svc.paraInfo(caller), { vchBytes: 0 }),
      parentHeadHash: headHash(svc.paraInfo(caller).headData),
      headData: svc.paraInfo(caller).headData, lookupAnchor: 0,
      upwardMessages: msgs })), authTrace: 0
  }
  action metadataBlock(budget: int, total: int, newHead: int): bool =
    storageInvocation(budget, List(metadataWork(CoretimeParaId, List(
      ParachainSetStateBalance({ paraId: MkParaId(3), newTotal: total }),
      ParachainSetHead({ paraId: MkParaId(3), newHead: newHead })))), List())

  // Empty final chunks abort and free the buffer; nonfinal chunks append.
  // Keep the total below overflow so every refusal here is a real host FULL.
  action stagingBlock(budget: int, count: int, abort: bool): bool =
    storageInvocation(budget, List(metadataWork(AssetHubParaId, List(
      SetValidatorKeys({ keys: if (abort) List() else range(0, count).foldl(List(),
        (a, i) => a.append(now + i)), isLast: abort })))), List())

  action metadataStep = {
    nondet budget = oneOf(Set(0, 1, 8, 60, 61, 1000))
    // Alternate across the compact-u64 boundary instead of getting stuck at a
    // large encoding where further balance updates cannot grow storage.
    val total = if (svc.isLive(MkParaId(3)) and
      svc.paraInfo(MkParaId(3)).totalStateBalance < 1073741824) 1073741824 else 1073741823
    nondet newHead = oneOf(Set(0, now + 1))
    metadataBlock(budget, total, newHead)
  }
  action stagingStep = {
    nondet budget = oneOf(Set(0, 335, 336, 337, 381, 382, 383, 1000, 22000))
    nondet count = oneOf(Set(0, 1, 3, 31))
    nondet abort = oneOf(Set(false, true))
    stagingBlock(budget, count, abort or svc.stagedValidatorKeys.length() > 64)
  }

  action forcedCodeBlock(margin: int): bool = {
    val target = MkParaId(4)
    val registered = parachainSetStateBalance(svc, target, 1073741823).state
    val acquired = registered.addReferencer(target, { hashBytes: 777 }, FixedCodeLen).state
    val budget = hostCost(acquired) - hostCost(svc) + margin
    storageInvocation(budget, List(metadataWork(CoretimeParaId, List(
      ParachainSetStateBalance({ paraId: target, newTotal: 1073741823 }),
      ParachainSetValidationCode({ paraId: target,
        newValidationCode: { hash: { vchBytes: 777 }, len: FixedCodeLen } })))), List())
  }
  action forcedCodeStep = {
    nondet margin = oneOf(Set(0, 35, 36))
    forcedCodeBlock(margin)
  }
}
