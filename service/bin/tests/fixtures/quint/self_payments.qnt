module self_payments {
  import outgoing_boundary_inputs as campaign from "./outgoing_boundary_inputs"
  run ordered_works = campaign::boundaryInit
    .then(campaign::selfPaymentAt(1000, -1, false))
    .then(campaign::selfPaymentAt(1000, 0, false))
    .then(campaign::selfPaymentAt(1000, -100, false))
    .then(campaign::outgoingBoundaryAt(1000, 0, false))
  run recovery_works = campaign::boundaryInit
    .then(campaign::selfPaymentAt(1000, 0, true))
    .then(campaign::selfPaymentAt(1000, -1, true))
    .then(campaign::selfPaymentAt(1000, 1, true))
    .then(campaign::selfPaymentAt(1000, -100, true))
    .then(campaign::outgoingBoundaryAt(1000, -1, false))
}
