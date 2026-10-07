// Replay ghost compatibility: implicit releases must end solicitation history.
module preimage_history {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import accumulate.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/accumulate"

  // Cleanup, forced code changes, and candidate code activation can release a
  // reference without an explicit Forget. Derive these events from the Quint
  // transition, never from observed Rust storage. Per-message ordering matters
  // when a para is removed and re-registered within the same work package.
  pure def releasedReferences(before: ParachainServiceState,
    after: ParachainServiceState): List[PreimageOp] =
    before.preimageRegistry.keys().fold(List(), (ops, key) =>
      before.preimageRegistry.get(key).referencers.fold(ops, (events, para) =>
        if (after.hasReferencer(key._1, key._2, para)) events
        else events.append(Forgotten((para, key._1, key._2)))))
}
