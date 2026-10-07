module self_payment_fuzz {
  import outgoing_boundary_inputs as campaign from "./outgoing_boundary_inputs"
  action replayInit = campaign::boundaryInit
  action replayStep = any { campaign::selfPaymentBlock, campaign::ordinaryOutgoing }
}
