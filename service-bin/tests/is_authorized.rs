//! `is_authorized` entry-point tests.
//!
//! The authorized `ParaId`s are sourced from the authorizer **config** (pinned by
//! the Coretime chain), not from the per-package authorization token: the service
//! requires every authorizer config to begin with a `Vec<ParaId>` (spec §3.2, §7.1).
//! These tests therefore exercise the config prefix; the token is left empty.

use codec::Encode;
use parachain_authorizer::ParaId;
use parachain_authorizer::{AuraAuthConfig, AuraCollatorAuthToken};
use parachain_authorizer_bin::BLOB as AUTHORIZER;
use primitive_types::H256;
use jam_types::{AuthConfig, Authorization as AuthToken, WorkItem, CodeHash};
use executor::jam::SERVICE_ID;

fn good_config(para_ids: usize) -> AuthConfig {
    let para_ids = (0..para_ids).map(|i| ParaId(i as u32)).collect::<Vec<_>>();
    let config = AuraAuthConfig {
        para_ids: para_ids.clone(),
        collator_set_root: H256::zero(),
        collator_set_size: 0,
        slot_duration: 0,
    };
    AuthConfig((para_ids, config).encode())
}

fn good_token() -> AuthToken {
    let token = AuraCollatorAuthToken {
        proof: vec![],
        key: vec![],
        signature: vec![],
    };
    AuthToken(token.encode())
}

fn work_items(n: usize) -> Vec<WorkItem> {
    (0..n).map(|i| WorkItem {
        service: SERVICE_ID,
        code_hash: CodeHash::zero(),
        refine_gas_limit: 0,
        accumulate_gas_limit: 0,
        export_count: 0,
        payload: Default::default(),
        import_segments: Default::default(),
        extrinsics: Default::default(),
    }).collect()
}

#[test]
fn trivial_works() {
    executor::jam::is_authorized(AUTHORIZER, good_config(1), good_token(), work_items(1), 0)
        .expect("is_authorized should run to completion (not trap)");
}

/// The spec of the authorizer enforces that the number of Para IDs must match the number of work
/// items, but it does not enforce it to be a single one. That is done by refine itself.
#[test]
fn two_work_items_works() {
    executor::jam::is_authorized(AUTHORIZER, good_config(2), good_token(), work_items(2), 0)
        .expect("is_authorized should run to completion (not trap)");
}

#[test]
fn more_work_items_than_para_ids_errors() {
    executor::jam::is_authorized(AUTHORIZER, good_config(1), good_token(), work_items(2), 0)
        .expect_err("is_authorized should error (not trap)");
}

#[test]
fn fewer_work_items_than_para_ids_errors() {
    executor::jam::is_authorized(AUTHORIZER, good_config(2), good_token(), work_items(1), 0)
        .expect_err("is_authorized should error (not trap)");
}

/// Empty work packages should be impossible per GP, but we still test it.
#[test]
fn no_work_items_errors() {
    executor::jam::is_authorized(AUTHORIZER, good_config(0), good_token(), work_items(0), 0)
        .expect_err("is_authorized should error (not trap)");
}

#[test]
fn config_trailing_data_errors() {
    let mut config = good_config(1);
    config.0.extend_from_slice(b"trailing data");

    executor::jam::is_authorized(AUTHORIZER, config, good_token(), work_items(1), 0)
        .expect_err("is_authorized should error (not trap)");
}

#[test]
fn token_trailing_data_errors() {
    let mut token = good_token();
    token.0.extend_from_slice(b"trailing data");

    executor::jam::is_authorized(AUTHORIZER, good_config(1), token, work_items(1), 0)
        .expect_err("is_authorized should error (not trap)");
}
