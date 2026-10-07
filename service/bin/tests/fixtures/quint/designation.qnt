module designation {
  import designation_inputs.* from "./designation_inputs"
  run privilege_recovery_works = start(0)
    .then(designationBlock(false, List(msg(3, 0, false))))
    .then(designationBlock(false, List(msg(3, 3, true))))
    .then(designationBlock(true, List(msg(6, 10, true))))
    .then(designationBlock(false, List(msg(6, 20, true))))
    .then(designationBlock(true, List(msg(6, 40, true))))
  run abort_and_invalid_works = start(6)
    .then(designationBlock(false, List(msg(0, 0, true))))
    .then(designationBlock(false, List(msg(1, 0, true))))
    .then(designationBlock(false, List(msg(3, 0, false))))
    .then(designationBlock(true, List(msg(3, 3, true))))
  run full_set_works = start(1000)
    .then(designationBlock(false, List(msg(23, 1000, true))))
    .then(designationBlock(true, List(msg(6, 0, true))))
}
