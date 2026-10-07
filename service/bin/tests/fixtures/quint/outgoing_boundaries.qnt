module outgoing_boundaries {
  import outgoing_boundary_inputs as campaign from "./outgoing_boundary_inputs"
  run below_works = campaign::boundaryInit
    .then(campaign::outgoingBoundaryAt(1000, -1, false))
    .then(campaign::outgoingBoundaryAt(100, -1, true))
    .then(campaign::outgoingBoundaryAt(1000, 1, false))
  run exact_works = campaign::boundaryInit
    .then(campaign::outgoingBoundaryAt(1000, 0, false))
    .then(campaign::outgoingBoundaryAt(100, 0, true))
    .then(campaign::outgoingBoundaryAt(1000, -1, false))
  run above_works = campaign::boundaryInit
    .then(campaign::outgoingBoundaryAt(1000, 1, false))
    .then(campaign::outgoingBoundaryAt(100, 1, true))
    .then(campaign::outgoingBoundaryAt(1000, 0, false))
}
