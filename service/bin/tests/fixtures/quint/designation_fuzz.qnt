module fuzz {
  import designation_inputs.* from "./designation_inputs"
  action replayInit = start(0)
  action replayStep = designationStep
}
