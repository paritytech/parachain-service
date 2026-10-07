module code_storage {
  import code_storage_inputs.* from "./code_storage_inputs"
  import types.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/types"
  import messages.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/messages"
  import main.* from "../../../../../vendor/polkadot-sdk-quint/designs/parachain-service-on-jam/quint/main"
  run acquisition_boundaries_works = codeInit
    .then(codeBlock(1000000, ParachainSetStateBalance({ paraId: MkParaId(3), newTotal: 1073741823 })))
    .then(codeBlock(0, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(1, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(2, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(85, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(86, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(65722, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(65723, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(35, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(36, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(0, forcedCode(MkParaId(3), 777)))
    .then(codeBlock(1000000, ParachainSetStateBalance({ paraId: MkParaId(4), newTotal: 1073741823 })))
    .then(codeBlock(3, forcedCode(MkParaId(4), 777)))
    .then(codeBlock(4, forcedCode(MkParaId(4), 777)))
    .then(codeBlock(36, forcedCode(MkParaId(4), 777)))
  run announcement_boundaries_works = codeInit
    .then(codeBlock(1000000, Solicit({ target: Parachain(CoretimeParaId), hash: { hashBytes: 777 }, len: FixedCodeLen })))
    .then(codeProvide(777))
    .then(codeBlock(0, announce(777)))
    .then(codeBlock(35, announce(777)))
    .then(codeBlock(36, announce(777)))
    .then(codeBlock(0, announce(777)))
}
