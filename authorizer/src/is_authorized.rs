use crate::ParaId;

use alloc::vec::Vec;
use jam_pvm_common::is_authorized::work_package;
use jam_types::{AuthTrace, Authorizer, CoreIndex};
use codec::Decode;

pub fn is_authorized(_core: CoreIndex) -> AuthTrace {
	// NOTE: We cannot use auth_config() here because PolkaJAM assumes specific encoding.
	let package = work_package();
	let Authorizer { config, .. } = package.authorizer;

	// We use `decode` and not `decode_all` since trailing config data (the AURA collator
	// set, slot timing, etc. — spec §7.1) is allowed after the `Vec<ParaId>` prefix.
	let _para_ids = Vec::<ParaId>::decode(&mut &config[..])
		.expect("the authorizer config must start with a list of para IDs");

	// PoC: echo the collator's authorization token back as the auth trace.
	AuthTrace(package.authorization.0)
}
