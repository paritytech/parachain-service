use super::{AuraAuthConfig, AuraCollatorAuthToken};
use codec::{DecodeAll, Encode};
use jam_pvm_common::is_authorized::{auth_token, work_package};
use jam_types::{AuthTrace, Authorizer, CoreIndex};

pub fn is_authorized(_core: CoreIndex) -> AuthTrace {
    let package = work_package();
    let Authorizer { config, .. } = &package.authorizer;

    let config = AuraAuthConfig::decode_all(&mut &config[..])
        .expect("authorizer config must decode to a (Vec<ParaId>, AuraAuthConfig)");

    if config.para_ids.len() != package.items.len() {
        panic!("auth config: number of para IDs does not match number of work items");
    }
    if package.items.len() == 0 {
        unreachable!("BUG: work packages need to have at least one item per Gray Paper");
    }

    let token = auth_token();
    let aura_token = AuraCollatorAuthToken::decode_all(&mut &token.0[..])
        .expect("the authorizer token must be a valid AuraCollatorAuthToken");

    let Some(trace) = aura_token.try_into_trace(&config, &package) else {
        panic!("the authorizer token is invalid");
    };

    // FIXME: Check the AURA round-robin collator selection

    AuthTrace(trace.author_key.encode())
}
