//! Outgoing transfers evaluated by the pinned Quint model.
use super::itf::replay;

#[test]
fn deferred_works() {
	replay::trace(include_str!("../fixtures/quint/outgoing/deferred_works.itf.json"))
		.expect("outgoing effects, balances, and logs should match Quint");
}

#[test]
fn zero_amount_works() {
	replay::trace(include_str!("../fixtures/quint/outgoing/zero_amount_works.itf.json"))
		.expect("outgoing effects, balances, and logs should match Quint");
}

#[test]
fn refusal_order_works() {
	replay::trace(include_str!("../fixtures/quint/outgoing/refusal_order_works.itf.json"))
		.expect("outgoing effects, balances, and logs should match Quint");
}

#[test]
fn gas_boundary_works() {
	replay::trace(include_str!("../fixtures/quint/outgoing/gas_boundary_works.itf.json"))
		.expect("outgoing effects, balances, and logs should match Quint");
}

#[test]
fn ordered_repeated_ids_works() {
	replay::trace(include_str!("../fixtures/quint/outgoing/ordered_repeated_ids_works.itf.json"))
		.expect("outgoing effects, balances, and logs should match Quint");
}

#[test]
fn skipped_work_works() {
	replay::trace(include_str!("../fixtures/quint/outgoing/skipped_work_works.itf.json"))
		.expect("outgoing effects, balances, and logs should match Quint");
}

#[test]
fn balance_carry_works() {
	replay::trace(include_str!("../fixtures/quint/outgoing/balance_carry_works.itf.json"))
		.expect("debits must persist across blocks, including a refused subsequent spend");
}
