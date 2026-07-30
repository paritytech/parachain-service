use crate::ParaId;

use alloc::vec::Vec;
use jam_pvm_common::is_authorized::{auth_token, auth_config};
use jam_types::{AuthTrace, Authorization, AuthConfig, CoreIndex};
use codec::Decode;

pub fn is_authorized(core: CoreIndex) -> AuthTrace {
	let AuthConfig(config) = auth_config();
	// We use `decode` and not `decode_all` since trailing data is allowed.
	let _para_ids = Vec::<ParaId>::decode(&mut &config[..]).expect("the authorizer config must start with a list of para IDs");

	let Authorization(token) = auth_token();

	AuthTrace(token)
}
