//! Assigning and freeing cores.
//!
//! Two lanes, and which one a command takes is decided by who currently holds the core's assigner
//! privilege, because that is the only service JAM will let assign it:
//!
//! - **bootstrap** — service 0, the assigner of every core at genesis. Instructions ride an
//!   unassigned core.
//! - **parasim** — a control package whose authorization token carries the command. Rides a core
//!   already under an AURA authorizer this tool can sign for, which need not be the target core.
//!
//! So the bootstrap order is: install the first AURA queue on a core, *then* hand that core's
//! assigner privilege to parasim. The other way round leaves a core parasim owns but cannot be
//! reached on — see the note in `grant`.

use jam_bootstrap_service_common::Instruction;
use jam_interface::JamChainSource;
use jam_rpc_interface::JamRpcInterface;
use jam_types::{AuthQueue, AuthorizerHash, CoreIndex, ServiceId};
use parachain_authorizer::aura::Command;
use parachain_service_interface::types::ParaId;

use crate::{
	aura::Aura,
	bootstrap,
	cores::{self, BOOTSTRAP_SERVICE},
	format::hex,
	package::{submit_and_follow, Anchor},
};

/// What every control command needs to know.
pub struct Args {
	/// The parasim service the AURA authorizer is configured for, and the assigner cores are
	/// handed to.
	pub service: ServiceId,
	pub aura: Aura,
	/// The core a control package rides, when one is needed. Defaults to the target core.
	pub via_core: Option<CoreIndex>,
	/// The para whose authorizer the carrier core is under, which is whose collator has to sign.
	pub via_para: ParaId,
}

/// Hand `core`'s assigner privilege to parasim, leaving its queue as it is.
///
/// This is the last bootstrap step for a core, not the first: afterwards service 0 can no longer
/// assign it, and parasim can only be reached through a core running the AURA authorizer. Granting
/// on a still-unassigned core therefore strands it — nothing on chain can assign it any more.
pub async fn grant(jam: &JamRpcInterface, args: &Args, core: CoreIndex) -> Result<(), String> {
	let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash;
	match cores::assigner(jam, best, core).await? {
		current if current == args.service => {
			println!("core {core} is already assigned by service {current}");
			return Ok(());
		},
		BOOTSTRAP_SERVICE => {},
		other =>
			return Err(format!(
				"core {core}'s assigner is service {other}, which is neither the bootstrap \
				 service nor parasim; nothing this tool can submit will move it"
			)),
	}

	// Keeping the queue means writing back what is already there. `Instruction::Assign` replaces
	// the whole queue, and a queue is 80 copies of one hash in this design, so re-filling it from
	// its own head changes nothing.
	let queue = cores::queue_head(jam, best, core).await?;
	if queue == cores::unassigned() {
		println!(
			"warning: core {core} is still unassigned. Once parasim owns it, only a control \
			 package on an AURA core can assign it — install its queue with `assign-core` first \
			 if this is the only core in play."
		);
	}
	bootstrap::instruct(
		jam,
		None,
		vec![Instruction::Assign { core, queue: AuthQueue::new(queue), assigner: args.service }],
	)
	.await?;

	let observed = wait_for_assigner(jam, core, args.service).await?;
	if observed != args.service {
		return Err(format!(
			"core {core}'s assigner is still service {observed}, expected {}",
			args.service
		));
	}
	println!("core {core} is now assigned by service {}", args.service);
	Ok(())
}

/// Point `core`'s authorizer queue at `para`'s AURA authorizer.
pub async fn assign(
	jam: &JamRpcInterface,
	args: &Args,
	para: ParaId,
	core: CoreIndex,
) -> Result<(), String> {
	let target = args.aura.hash(para);
	println!("core {core} → para {} (authorizer 0x{})", para.0, hex(&target.0));
	route(jam, args, core, target, Command::Assign { para_id: para, core, authorizer: target.0 })
		.await?;
	cores::report(jam, core, target).await
}

/// Return `core` to the unassigned authorizer, draining its pool over the next few blocks.
pub async fn free(jam: &JamRpcInterface, args: &Args, core: CoreIndex) -> Result<(), String> {
	let target = cores::unassigned();
	println!("core {core} → unassigned (authorizer 0x{})", hex(&target.0));
	route(jam, args, core, target, Command::Free { core }).await?;
	cores::report(jam, core, target).await
}

/// Send `command` down whichever lane the core's assigner leaves open.
async fn route(
	jam: &JamRpcInterface,
	args: &Args,
	core: CoreIndex,
	target: AuthorizerHash,
	command: Command,
) -> Result<(), String> {
	let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash;
	match cores::assigner(jam, best, core).await? {
		BOOTSTRAP_SERVICE => {
			println!("core {core} is still assigned by the bootstrap service; going through it");
			bootstrap::instruct(
				jam,
				None,
				vec![Instruction::Assign {
					core,
					queue: AuthQueue::new(target),
					assigner: BOOTSTRAP_SERVICE,
				}],
			)
			.await
		},
		assigner if assigner == args.service => control_package(jam, args, core, command).await,
		other => Err(format!(
			"core {core}'s assigner is service {other}, not the bootstrap service and not parasim \
			 ({}); nothing this tool can submit will assign it",
			args.service
		)),
	}
}

/// Submit a package whose only job is to carry `command` in its authorization token.
///
/// The work item is parasim's control no-op: a control package has no parachain block in it, and
/// the command reaches accumulate through the authorization output, not through the item.
async fn control_package(
	jam: &JamRpcInterface,
	args: &Args,
	target_core: CoreIndex,
	command: Command,
) -> Result<(), String> {
	let core = args.via_core.unwrap_or(target_core);
	let anchor = Anchor::fetch(jam, args.service).await?;

	// The carrier core has to be under the very authorizer this token is built for, or the
	// package is refused before parasim ever sees it — and a refused package is one more thing
	// that looks like a silent failure.
	let expected = args.aura.hash(args.via_para);
	let head = cores::queue_head(jam, anchor.context.anchor, core).await?;
	if head != expected {
		return Err(format!(
			"core {core} holds authorizer 0x{}, but a token for para {} hashes to 0x{}. Name the \
			 para that core is actually running with --via-para, or a core that is running one \
			 with --via-core.",
			hex(&head.0),
			args.via_para.0,
			hex(&expected.0),
		));
	}
	println!("carrying the command on core {core}, under para {}'s authorizer", args.via_para.0);

	let mut package = anchor.package(
		args.aura.authorizer(args.via_para),
		vec![anchor.item(args.service, parasim_service::CONTROL_NOOP_PAYLOAD.to_vec())],
	);
	package.authorization = args.aura.token(&package, Some(command))?;
	submit_and_follow(jam, core, &package).await
}

/// Wait for `core`'s assigner privilege to reach `expected`, returning who holds it in the end.
async fn wait_for_assigner(
	jam: &JamRpcInterface,
	core: CoreIndex,
	expected: ServiceId,
) -> Result<ServiceId, String> {
	const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
	const POLL: std::time::Duration = std::time::Duration::from_secs(3);

	let deadline = tokio::time::Instant::now() + TIMEOUT;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let assigner = cores::assigner(jam, best.header_hash, core).await?;
		if assigner == expected || tokio::time::Instant::now() >= deadline {
			return Ok(assigner);
		}
		tokio::time::sleep(POLL).await;
	}
}
