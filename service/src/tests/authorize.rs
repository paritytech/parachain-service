//! `is_authorized` entry-point tests.

use executor::jam;

use super::authorizer_blob;

#[test]
fn is_authorized_echoes_token_as_auth_trace() {
    let authorizer = authorizer_blob();
    let token = b"parachain-auth-token".to_vec();

    let outcome = jam::is_authorized(&authorizer, token.clone(), 0)
        .expect("is_authorized should run to completion (not trap)");

    // The parachain authorizer authorizes unconditionally and forwards its input
    // token verbatim as the auth trace handed to refine/accumulate.
    assert_eq!(
        outcome.auth_trace.0, token,
        "auth trace should echo the token"
    );
    println!(
        "is_authorized ok in {:?}, gas used {}, trace: {} bytes",
        outcome.elapsed,
        outcome.gas_used,
        outcome.auth_trace.0.len()
    );
}
