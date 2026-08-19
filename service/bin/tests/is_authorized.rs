//! `is_authorized` entry-point tests.
//!
//! The authorized `ParaId`s are sourced from the authorizer **config** (pinned by
//! the Coretime chain), not from the per-package authorization token: the service
//! requires every authorizer config to begin with a `Vec<ParaId>` (spec §3.2, §7.1).

use executor::pj;
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::mock::{
	good_config, good_token, is_authorized_args, make_auth,
	make_auth_with_seed, make_single_collator_args_with_key, make_wrong_collator_index_args,
	work_items, MOCK_SERVICE_ID,
};
use parachain_service_interface::types::ParaId;

#[test]
fn trivial_works() {
	let items = work_items(1);
	let (config, token, _) = make_auth(AUTHORIZER, vec![ParaId(0)], &items);
	let (engine, package, storage) = is_authorized_args(AUTHORIZER, config, token, items);
	pj::is_authorized(&engine, &package, 0, &storage)
		.expect("is_authorized should run to completion (not trap)");
}

/// The spec of the authorizer enforces that the number of Para IDs must match the number of work
/// items, but it does not enforce it to be a single one. That is done by refine itself.
#[test]
fn two_work_items_works() {
	let items = work_items(2);
	let (config, token, _) = make_auth(AUTHORIZER, vec![ParaId(0), ParaId(1)], &items);
	let (engine, package, storage) = is_authorized_args(AUTHORIZER, config, token, items);
	pj::is_authorized(&engine, &package, 0, &storage)
		.expect("is_authorized should run to completion (not trap)");
}

#[test]
fn more_work_items_than_para_ids_errors() {
	let (engine, package, storage) =
		is_authorized_args(AUTHORIZER, good_config(1), good_token(), work_items(2));

	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("is_authorized should error (not trap)");
}

#[test]
fn fewer_work_items_than_para_ids_errors() {
	let (engine, package, storage) =
		is_authorized_args(AUTHORIZER, good_config(2), good_token(), work_items(1));

	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("is_authorized should error (not trap)");
}

/// Empty work packages should be impossible per GP, but we still test it.
#[test]
fn no_work_items_errors() {
	let (engine, package, storage) =
		is_authorized_args(AUTHORIZER, good_config(0), good_token(), work_items(0));

	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("is_authorized should error (not trap)");
}

#[test]
fn config_trailing_data_errors() {
	let mut config = good_config(1);
	config.0.extend_from_slice(b"trailing data");
	let (engine, package, storage) =
		is_authorized_args(AUTHORIZER, config, good_token(), work_items(1));

	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("is_authorized should error (not trap)");
}

#[test]
fn token_trailing_data_errors() {
	let mut token = good_token();
	token.0.extend_from_slice(b"trailing data");
	let (engine, package, storage) =
		is_authorized_args(AUTHORIZER, good_config(1), token, work_items(1));

	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("is_authorized should error (not trap)");
}

/// [0u8; 32] decodes as the Edwards compressed point y=0 (order-4 torsion).
/// verify_strict rejects it — this test is the executable proof that
/// check_signature uses verify_strict and not the weaker verify.
#[test]
fn small_order_key_errors() {
	let small_order_key = [0u8; 32];
	let (engine, package, storage) = make_single_collator_args_with_key(
		AUTHORIZER,
		vec![ParaId(0)],
		work_items(1),
		small_order_key,
		[0u8; 64],
	);
	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("small-order collator key must be rejected by verify_strict");
}

/// [2u8; 32] is used to test the bad-key rejection path — either
/// VerifyingKey::from_bytes fails (not a valid compressed Edwards point) or
/// the zero signature fails verify_strict. Either way authorization must fail.
#[test]
fn undecodable_collator_key_errors() {
	let bad_key = [2u8; 32];
	let (engine, package, storage) = make_single_collator_args_with_key(
		AUTHORIZER,
		vec![ParaId(0)],
		work_items(1),
		bad_key,
		[0u8; 64],
	);
	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("non-curve-point collator key must fail authorization");
}

/// A real valid Merkle proof for collator 1 is paired with the expected
/// collator index 0 (slot=0, set_size=2 → index 0). The proof walk
/// reconstructs the wrong root → BadCollatorSetProof. This proves the stub
/// was replaced with a real Merkle check that can detect index mismatches.
#[test]
fn proof_for_wrong_index_errors() {
	let seeds = [[0x42u8; 32], [0x43u8; 32]];
	let (engine, package, storage) = make_wrong_collator_index_args(
		AUTHORIZER,
		vec![ParaId(0)],
		work_items(1),
		&seeds,
		1,
	);
	pj::is_authorized(&engine, &package, 0, &storage)
		.expect_err("Merkle proof for index 1 must be rejected for expected index 0");
}

/// RFC 8032 §7.1 Test Vector 1 keypair run end-to-end through the real
/// authorizer PVM blob. This proves the guest-side curve25519-dalek backend
/// computes correct ed25519 arithmetic on riscv64emac-unknown-none-polkavm.
#[test]
fn ed25519_known_answer_vector_works() {
	let rfc8032_seed: [u8; 32] = [
		0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
		0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
		0x1c, 0xae, 0x7f, 0x60,
	];
	let expected_vk: [u8; 32] = [
		0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
		0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
		0xf7, 0x07, 0x51, 0x1a,
	];

	let items = work_items(1);
	let (config, token, _) = make_auth_with_seed(AUTHORIZER, vec![ParaId(0)], &items, rfc8032_seed);
	let (engine, package, storage) = is_authorized_args(AUTHORIZER, config, token, items);
	let outcome = pj::is_authorized(&engine, &package, 0, &storage)
		.expect("RFC 8032 §7.1 Test Vector 1 must pass is_authorized through the PVM");
	eprintln!("ed25519_known_answer_vector_works: gas_used={}", outcome.gas_used);

	use codec::DecodeAll;
	let trace = parachain_authorizer::aura::AuthTrace::decode_all(&mut &outcome.auth_trace.0[..])
		.expect("auth trace must decode");
	assert_eq!(trace.author_key, expected_vk, "author key must match RFC 8032 verifying key");

	let _ = MOCK_SERVICE_ID;
}
