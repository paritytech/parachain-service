use super::aura;
use codec::{DecodeAll, Encode};
use jam_pvm_common::is_authorized::{auth_token, work_package};
use jam_types::{AuthTrace, CoreIndex};

#[derive(Debug)]
pub enum AuthorizationError {
    UndecodableAuthConfig,
    UndecodableAuthToken,
    InvalidWorkItemCount,
    BadAuthToken(aura::AuthorizationError),
}

pub fn is_authorized(_core: CoreIndex) -> Result<AuthTrace, AuthorizationError> {
    let package = work_package();
    let auth_config = &package.authorizer.config;

    let config = aura::AuthConfig::decode_all(&mut &auth_config[..])
        .map_err(|_| AuthorizationError::UndecodableAuthConfig)?;

    if config.para_ids.len() != package.items.len() {
        return Err(AuthorizationError::InvalidWorkItemCount);
    }
    assert!(
        package.items.len() > 0,
        "work packages need to have at least one item per Gray Paper"
    );

    let token = auth_token();
    let aura_token = aura::AuthToken::decode_all(&mut &token.0[..])
        .map_err(|_| AuthorizationError::UndecodableAuthToken)?;

    let trace = aura_token
        .try_into_trace(&config, &package)
        .map_err(|e| AuthorizationError::BadAuthToken(e))?;

    // FIXME: Check the AURA round-robin collator selection

    Ok(AuthTrace(trace.author_key.encode()))
}
