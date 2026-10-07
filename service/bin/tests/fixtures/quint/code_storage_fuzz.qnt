module fuzz {
  import code_storage_inputs.* from "./code_storage_inputs"
  action replayInit = codeInit
  action replayStep = any { codeStep, codeProvision }
}
