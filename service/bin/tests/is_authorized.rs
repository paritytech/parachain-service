//! `is_authorized` entry-point tests.
//!
//! The authorized `ParaId`s are sourced from the authorizer **config** (pinned by
//! the Coretime chain), not from the per-package authorization token: the service
//! requires every authorizer config to begin with a `Vec<ParaId>` (spec §3.2, §7.1).
//! These tests therefore exercise the config prefix; the token is left empty.

use executor::pj;
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::mock::{good_config, good_token, is_authorized_args, work_items};

#[test]
fn trivial_works() {
    let (engine, package, storage) =
        is_authorized_args(AUTHORIZER, good_config(1), good_token(), work_items(1));

    pj::is_authorized(&engine, &package, 0, &storage)
        .expect("is_authorized should run to completion (not trap)");
}

/// The spec of the authorizer enforces that the number of Para IDs must match the number of work
/// items, but it does not enforce it to be a single one. That is done by refine itself.
#[test]
fn two_work_items_works() {
    let (engine, package, storage) =
        is_authorized_args(AUTHORIZER, good_config(2), good_token(), work_items(2));

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
