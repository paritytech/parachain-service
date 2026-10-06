//! Incoming transfers and work reports execute in the same PVM invocation.
use crate::itf::replay;

#[test]
fn arrivals_and_reports_works() {
	replay::trace(include_str!("../fixtures/quint/mixed/arrivals_and_reports_works.itf.json"))
		.expect("mixed invocation should match Quint storage, balances, and effects");
}

#[test]
fn arrivals_and_rejected_reports_works() {
	replay::trace(include_str!(
		"../fixtures/quint/mixed/arrivals_and_rejected_reports_works.itf.json"
	))
	.expect("mixed invocation should match Quint storage, balances, and effects");
}

#[test]
fn mixed_bucket_boundary_works() {
	replay::trace(include_str!("../fixtures/quint/mixed/mixed_bucket_boundary_works.itf.json"))
		.expect("mixed invocation should match Quint storage, balances, and effects");
}

#[test]
fn incoming_funds_outgoing_works() {
	replay::trace(include_str!("../fixtures/quint/mixed/incoming_funds_outgoing_works.itf.json"))
		.expect("mixed invocation should match Quint storage, balances, and effects");
}

#[test]
fn due_assignment_and_arrivals_works() {
	replay::trace(include_str!(
		"../fixtures/quint/mixed/due_assignment_and_arrivals_works.itf.json"
	))
	.expect("mixed invocation should match Quint storage, balances, and effects");
}
