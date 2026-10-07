module outgoing_boundary_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import support.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/tests/support"
  import fuzz.* from "./fuzz"
  import gas_inputs.* from "./gas_inputs"
  import transfer_inputs.* from "./transfer_inputs"
  import validator_key_inputs.* from "./validator_key_inputs"
  import outgoing_inputs.* from "./outgoing_inputs"
  import service_upgrade_inputs.* from "./service_upgrade_inputs"
  import host_invocation.* from "./host_invocation"
  action boundaryInit = replayInit
  action ordinaryOutgoing = outgoingBlock
  // The first report normalizes head/log storage before calculating the
  // exact spendable balance for the second report's transfer.
  action outgoingBoundaryAt(allowance: int, adjacent: int, stop: bool): bool =
    paymentBoundaryAt(allowance, adjacent, stop, false)
  action selfPaymentAt(allowance: int, adjacent: int, stop: bool): bool =
    paymentBoundaryAt(allowance, adjacent, stop, true)
  action paymentBoundaryAt(allowance: int, adjacent: int, stop: bool, self: bool): bool = {
    val para = AssetHubParaId
    val first = replayWork(svc, para, svc.paraInfo(para).headData,
      activeCodeCandidate(svc, para), 0, now, 0, List())
    val prefix = hostPrefix(List(first), now + 1, List(), List(UnlimitedGas), allowance)
    val payment = TransferOut({ ...transfer(7, prefix.free + adjacent, 71, 200),
      dest: if (self) ParachainServiceId else MkServiceId(7) })
    // A following spend cannot use a self-payment's not-yet-delivered credit.
    val payments = if (self) List(payment, TransferOut(transfer(7, 1, 73, 200))) else List(payment)
    val second = replayWork(prefix.state, para, prefix.state.paraInfo(para).headData,
      activeCodeCandidate(prefix.state, para), 0, now, 0, payments)
    val after = hostPrefix(List(first, second), now + 1, List(), List(UnlimitedGas, UnlimitedGas), allowance)
    val fault = replayWork(after.state, para, after.state.paraInfo(para).headData,
      activeCodeCandidate(after.state, para), 0, now, 0,
      (if (self) List(TransferOut({ ...transfer(7, 50, 74, 200), dest: ParachainServiceId }))
        else List()).append(TransferOut(transfer(7, 0, 72, 6000000000))))
    all {
      prefix.free >= 1 and prefix.free + adjacent >= 0,
      hostInvocation(if (stop) List(first, second, fault) else List(first, second),
        now + 1, List(), if (stop) List(UnlimitedGas, UnlimitedGas, UnlimitedGas)
          else List(UnlimitedGas, UnlimitedGas), allowance, if (stop) 2 else -1)
    }
  }
  action outgoingBoundaryBlock = {
    nondet allowance = oneOf(Set(100, 1000, 4096))
    nondet adjacent = oneOf(Set(-1, 0, 1))
    nondet stop = oneOf(Set(false, true))
    outgoingBoundaryAt(allowance, adjacent, stop)
  }

  action selfPaymentBlock = {
    nondet allowance = oneOf(Set(100, 1000, 4096))
    nondet adjacent = oneOf(Set(-100, -1, 0, 1))
    nondet stop = oneOf(Set(false, true))
    selfPaymentAt(allowance, adjacent, stop)
  }
}
