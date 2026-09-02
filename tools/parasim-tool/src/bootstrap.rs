//! The bootstrap-service lane: work packages addressed to service 0.
//!
//! Service 0 is manager, registrar, designator and assigner of every core at genesis, and its
//! work-item payload is just a list of instructions. That makes it the way in for everything that
//! has to happen before parasim owns anything: soliciting the authorizer blob, and handing over
//! the assigner privilege.
//!
//! It only rides an *unassigned* core. Once a core is under the AURA authorizer, that authorizer
//! refuses any work item not addressed to the parachain service — which is exactly what it is
//! for, and the reason the bootstrap steps have to come first.

use jam_bootstrap_service_common::{Instruction, Instructions, PayloadSalt};
use jam_interface::{JamChainSource, JamStateSource};
use jam_rpc_interface::JamRpcInterface;
use jam_types::{AuthConfig, Authorizer, CoreIndex};

use crate::{
	cores::{self, BOOTSTRAP_SERVICE},
	format::hex,
	package::{submit_and_follow, Anchor},
};

/// Submit `instructions` to the bootstrap service and wait for JAM to report the package.
///
/// `core` defaults to the lowest unassigned core, since that is the only kind this can ride.
pub async fn instruct(
	jam: &JamRpcInterface,
	core: Option<CoreIndex>,
	instructions: Vec<Instruction>,
) -> Result<(), String> {
	let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash;
	let core = match core {
		Some(core) => {
			let head = cores::queue_head(jam, best, core).await?;
			if head != cores::unassigned() {
				return Err(format!(
					"core {core} is under authorizer 0x{}, not the unassigned one — a bootstrap \
					 instruction only rides an unassigned core",
					hex(&head.0)
				));
			}
			core
		},
		None => unassigned_core(jam, best).await?,
	};

	let anchor = Anchor::fetch(jam, BOOTSTRAP_SERVICE).await?;
	// The bootstrap service's payload is jam-codec, not the SCALE `codec` the parachain wire
	// types use; the two are not interchangeable.
	let payload =
		jam_codec::Encode::encode(&Instructions { instructions, salt: PayloadSalt::random() });
	let package = anchor.package(
		Authorizer {
			code_hash: jam_null_authorizer_bin::HASH.into(),
			config: AuthConfig::default(),
		},
		vec![anchor.item(BOOTSTRAP_SERVICE, payload)],
	);
	submit_and_follow(jam, core, &package).await
}

/// The lowest core still holding the unassigned authorizer.
async fn unassigned_core(
	jam: &JamRpcInterface,
	at: jam_interface::HeaderHash,
) -> Result<CoreIndex, String> {
	let queues = jam.auth_queues(at).await.map_err(|e| format!("reading the queues: {e}"))?;
	queues
		.iter()
		.position(|queue| queue.get(0) == Some(&cores::unassigned()))
		.map(|core| core as CoreIndex)
		.ok_or_else(|| {
			"every core is assigned, so no core will authorize a bootstrap instruction. Free a \
			 core parasim already owns (`free-core <core> --via-core <other>`), run this again, \
			 then assign that core back through the same carrier."
				.to_string()
		})
}
