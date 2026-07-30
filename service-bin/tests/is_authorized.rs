//! `is_authorized` entry-point tests.

use codec::Encode;
use executor::jam;
use parachain_authorizer::ParaId;
use parachain_authorizer_bin::BLOB as AUTHORIZER;

#[test]
fn token_not_starting_with_para_ids_errors() {
    let token = b"not-starting-with-parids".to_vec();

    jam::is_authorized(AUTHORIZER, token, 0).expect_err("is_authorized should error (not trap)");
}

#[test]
fn token_single_paraid_works() {
    let token = vec![ParaId(1)].encode();

    jam::is_authorized(AUTHORIZER, token, 0).expect("is_authorized should run to completion (not trap)");
}

#[test]
fn token_multiple_paraids_works() {
    let token = vec![ParaId(1), ParaId(2)].encode();

    jam::is_authorized(AUTHORIZER, token, 0).expect("is_authorized should run to completion (not trap)");
}

#[test]
fn token_trailing_data_works() {
    let mut token = vec![ParaId(1), ParaId(2)].encode();
    token.extend_from_slice(b"trailing data");

    jam::is_authorized(AUTHORIZER, token, 0).expect("is_authorized should run to completion (not trap)");
}

#[test]
fn token_wrong_para_id_type_errors() {
    let token = vec![1u16].encode();

    jam::is_authorized(AUTHORIZER, token, 0).expect_err("is_authorized should error (not trap)");
}
