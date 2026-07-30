//! `is_authorized` entry-point tests.
//!
//! The authorized `ParaId`s are sourced from the authorizer **config** (pinned by
//! the Coretime chain), not from the per-package authorization token: the service
//! requires every authorizer config to begin with a `Vec<ParaId>` (spec §3.2, §7.1).
//! These tests therefore exercise the config prefix; the token is left empty.

use codec::Encode;
use executor::jam;
use parachain_authorizer::ParaId;
use parachain_authorizer_bin::BLOB as AUTHORIZER;

#[test]
fn config_not_starting_with_para_ids_errors() {
    let config = b"not-starting-with-parids".to_vec();

    jam::is_authorized(AUTHORIZER, config, Vec::new(), 0)
        .expect_err("is_authorized should error (not trap)");
}

#[test]
fn config_single_paraid_works() {
    let config = vec![ParaId(1)].encode();

    jam::is_authorized(AUTHORIZER, config, Vec::new(), 0)
        .expect("is_authorized should run to completion (not trap)");
}

#[test]
fn config_multiple_paraids_works() {
    let config = vec![ParaId(1), ParaId(2)].encode();

    jam::is_authorized(AUTHORIZER, config, Vec::new(), 0)
        .expect("is_authorized should run to completion (not trap)");
}

#[test]
fn config_trailing_data_works() {
    // The real config carries `collator_set_root`, `collator_set_size`, etc. after the
    // `Vec<ParaId>` prefix; decoding uses `decode` (not `decode_all`) so that is allowed.
    let mut config = vec![ParaId(1), ParaId(2)].encode();
    config.extend_from_slice(b"trailing data");

    jam::is_authorized(AUTHORIZER, config, Vec::new(), 0)
        .expect("is_authorized should run to completion (not trap)");
}

#[test]
fn config_wrong_para_id_type_errors() {
    let config = vec![1u16].encode();

    jam::is_authorized(AUTHORIZER, config, Vec::new(), 0)
        .expect_err("is_authorized should error (not trap)");
}
