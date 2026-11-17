// Generated charge-width crossings. A solicitation reserves bytes without
// providing them, so even the 2^40 boundary needs no large byte allocation.
module kv_charge_fuzz {
  import host_mixed.* from "./host_mixed"
  import fuzz as common from "./fuzz"
  import host_invocation.* from "./host_invocation"
  import gas_inputs.* from "./gas_inputs"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  var chargePhase: int
  var chargeWidth: int
  var chargeOldLen: int
  var chargeGrowth: int
  action replayInit = {
    nondet boundary = Set(1073741824, 4294967296, 1099511627776).oneOf()
    nondet oldLen = Set(0, 1).oneOf()
    nondet growth = Set(1, 2, 64).oneOf()
    all { common::replayInit, chargePhase' = 0, chargeWidth' = boundary,
      chargeOldLen' = oldLen, chargeGrowth' = growth }
  }
  val required = 1 + chargeGrowth + (if (chargeOldLen == 0) 50 else 0)
  action shrink(stop: int): bool = hostInvocation(List(common::replayWork(svc, AssetHubParaId,
    svc.paraInfo(AssetHubParaId).headData,
    common::activeCodeCandidate(svc, AssetHubParaId), 0, 0, 0,
    List(if (chargeOldLen == 0)
      RemoveKV({ paraId: AssetHubParaId, key: List(83) })
      else storeKV(83, chargeOldLen)))), now + 1,
    List(), List(UnlimitedGas), 0, stop)
  // Split the reservation into u32-sized preimages with distinct hashes.
  // Each request charges 187 + len; distribute the remainder one unit per request.
  val requestCount = if (chargeWidth <= 4294967296) 1 else 256
  val reservation = chargeWidth - 1 - svc.paraInfo(AssetHubParaId).usedStateBalance
  val requestCharge = reservation / requestCount
  val requests = range(0, requestCount).foldl(List(), (list, i) =>
    list.append(solicit(10000 + i,
      requestCharge + (if (i < reservation % requestCount) 1 else 0) - 187)))
  action replayStep = all {
    chargeWidth' = chargeWidth, chargeOldLen' = chargeOldLen,
    chargeGrowth' = chargeGrowth, chargePhase' = chargePhase + 1,
    if (chargePhase == 0)
      normal(CoretimeParaId, List(ParachainSetStateBalance({
        paraId: AssetHubParaId, newTotal: 2 * chargeWidth })))
    else if (chargePhase == 1)
      normal(AssetHubParaId,
        if (chargeOldLen == 0) List() else List(storeKV(83, chargeOldLen)))
    else if (chargePhase == 2)
      normal(AssetHubParaId, requests)
    else if (chargePhase == 3) chargeWrite(0, chargeOldLen + chargeGrowth, 1)
    else if (chargePhase == 4) chargeWrite(1, chargeOldLen + chargeGrowth, 1)
    else if (chargePhase == 5) chargeWrite(required - 1, chargeOldLen + chargeGrowth, 1)
    else if (chargePhase == 6) chargeWrite(required, chargeOldLen + chargeGrowth, 0)
    else if (chargePhase == 7) chargeWrite(required, chargeOldLen + chargeGrowth, 1)
    else if (chargePhase == 8) shrink(1)
    else if (chargePhase == 9) chargeWrite(required, chargeOldLen + chargeGrowth, -1)
    else shrink(-1)
  }
}
