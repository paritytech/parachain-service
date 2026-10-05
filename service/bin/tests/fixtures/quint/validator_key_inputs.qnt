// Bounded validator-key inputs for streaming replay; outcomes come from the pin.
module validator_key_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"

  var replayDesignate: List[ValidatorKey]
  action noDesignation = replayDesignate' = List()

  // Empty aborts, valid/invalid final lengths, and the Refine chunk-size boundary.
  // Two overlapping key ranges also exercise ordering and repeated designations.
  pure val validatorKeyMessages: Set[UpwardMessage] =
    Set(0, 1, 3, 5, 6, 30, 31).fold(Set(), (msgs, n) =>
      Set(0, 10).fold(msgs, (acc, start) =>
        Set(false, true).fold(acc, (out, last) => out.union(Set(
          SetValidatorKeys({
            keys: range(0, n).foldl(List(), (chunk, i) => chunk.append(start + i)),
            isLast: last
          })
        )))))
}
