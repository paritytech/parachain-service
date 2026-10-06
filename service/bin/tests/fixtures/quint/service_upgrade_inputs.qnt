// Two executable, metadata-distinct copies of the service, padded by the Rust
// codex to this exact length. No random non-executable blob may be installed.
module service_upgrade_inputs {
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import state.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state"
  import state_vars.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/state_vars"
  pure val ServiceCodeLen = 262144
  pure val serviceCodes: Set[Hash] = Set({ hashBytes: 9001 }, { hashBytes: 9002 })
  var replayMinAccGas: int
  action keepUpgradeGas = replayMinAccGas' = replayMinAccGas
  pure def upgrade(hash: Hash, acc: int, memo: int): UpwardMessage =
    UpgradeService({ codeHash: hash, len: ServiceCodeLen, minAccGas: acc, minMemoGas: memo })
  pure def serviceUpgradeMessages: Set[UpwardMessage] = serviceCodes.fold(Set(), (msgs, hash) =>
    msgs.union(Set(
      Solicit({ target: Parachain(AssetHubParaId), hash: hash, len: ServiceCodeLen }),
      Solicit({ target: Parachain(CoretimeParaId), hash: hash, len: ServiceCodeLen }),
      Forget({ target: Parachain(AssetHubParaId), hash: hash, len: ServiceCodeLen }),
      Forget({ target: Parachain(CoretimeParaId), hash: hash, len: ServiceCodeLen }),
      upgrade(hash, 0, 0), upgrade(hash, 100, 200), upgrade(hash, 123456, 789),
      UpgradeService({ codeHash: hash, len: ServiceCodeLen + 1, minAccGas: 55, minMemoGas: 66 })
    )))
}
