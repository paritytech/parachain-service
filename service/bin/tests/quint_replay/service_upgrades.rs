//! Service upgrades evaluated by Quint and executed by the installed PVM code.
use super::itf::replay;

#[test]
fn activation_works() {
	replay::trace(include_str!("../fixtures/quint/service_upgrades/activation_works.itf.json"))
		.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn missing_and_wrong_length_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_upgrades/missing_and_wrong_length_works.itf.json"
	))
	.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn unreferenced_works() {
	replay::trace(include_str!("../fixtures/quint/service_upgrades/unreferenced_works.itf.json"))
		.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn forgotten_and_rerequested_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_upgrades/forgotten_and_rerequested_works.itf.json"
	))
	.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn running_code_protected_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_upgrades/running_code_protected_works.itf.json"
	))
	.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn same_code_new_gas_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_upgrades/same_code_new_gas_works.itf.json"
	))
	.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn ordered_upgrades_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_upgrades/ordered_upgrades_works.itf.json"
	))
	.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn rejected_work_works() {
	replay::trace(include_str!("../fixtures/quint/service_upgrades/rejected_work_works.itf.json"))
		.expect("service code, gas settings, and preimages should match Quint");
}

#[test]
fn same_package_guard_works() {
	replay::trace(include_str!(
		"../fixtures/quint/service_upgrades/same_package_guard_works.itf.json"
	))
	.expect("running-code protection must follow upgrades within a package");
}
