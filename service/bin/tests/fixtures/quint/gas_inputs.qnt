// Replay-only gas vocabulary: the pinned model abstracts execution budgets.
module gas_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"
  import host_metadata.* from "./host_metadata"
  var replayGasLimits: List[int]
  // -2 fails before the first checkpoint; -1 completes; 0..length-1 interrupts that report; length fails after the final checkpoint.
  var replayInterrupt: int
  // Inject an undecodable work digest at replayInterrupt to trigger a real PVM trap.
  var replayPanic: bool
  pure val UnlimitedGas = 18446744073709551615
  pure val ExhaustPoolGas = 6000000000
  action noGas = all { noHostBudget, replayGasLimits' = List(), replayInterrupt' = -1, replayPanic' = false }

  // §5.1 implementation budget, including gas forwarded by deferred transfers.
  // Keep independent of Rust report_cost so boundary replays detect drift.
  pure def reportGas(wr: WorkResult): int = match wr.result {
    | WorkErr => 0
    | WorkOk(digest) => match digest {
      | Err(_) => 5000000
      | Ok(ok) => ok.upwardMessages.foldl(5000000, (cost, msg) => {
        val forwarded = match msg {
          | TransferOut(t) => match t.deferred {
            | None => 0
            | Some(pair) => pair._2
          }
          | _ => 0
        }
        val total = cost + 250000 + forwarded
        if (total > UnlimitedGas) UnlimitedGas else total
      })
    }
  }
  pure def budget(wr: WorkResult, mode: int): int = {
    val cost = reportGas(wr)
    if (mode == 0) 0 else if (mode == 1) (if (cost > 0) cost - 1 else 0)
    else if (mode == 2) cost else if (mode == 3) (if (cost < UnlimitedGas) cost + 1 else cost) else UnlimitedGas
  }
}
