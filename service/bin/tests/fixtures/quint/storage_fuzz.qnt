module fuzz {
  import storage_inputs.* from "./storage_inputs"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  import storage_metadata_inputs.* from "./storage_metadata_inputs"
  action replayInit = storageInit
  action queueStep = {
    nondet budget = oneOf(Set(0, 7, 8, 50, 58, 59, 90, 187, 188, 253, 254, 500, 1000))
    nondet count = oneOf(Set(0, 1, 2, 3))
    nondet amount = oneOf(Set(0, 1, 63, 64))
    nondet len = oneOf(Set(0, 1, 63, 64, 256))
    nondet newHead = oneOf(Set(0, now + 1))
    nondet chain = oneOf(Set(false, true))
    storageBlock(budget, if (queuedTransferCount(svc) < 1000) count else 0, amount, len, newHead, chain)
  }
  action replayStep = any { queueStep, metadataStep, stagingStep, forcedCodeStep }
}
