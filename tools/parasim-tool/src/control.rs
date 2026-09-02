//! Assigning and freeing cores.
//!
//! Two lanes, and which one a command takes is decided by who currently holds the core's assigner
//! privilege, because that is the only service JAM will let assign it:
//!
//! - **bootstrap** — service 0, the assigner of every core at genesis. Instructions ride an
//!   unassigned core.
//! - **parasim** — a control package carrying the command as its work-item payload. Rides a core
//!   already under an AURA authorizer this tool can name, which need not be the target core, and
//!   may be a parked one: parking keeps the authorizer, so a parked core still takes commands.
//!
//! So the bootstrap order is: install the first AURA queue on a core, *then* hand that core's
//! assigner privilege to parasim. The other way round leaves a core parasim owns but cannot be
//! reached on — see the note in `grant`.

use codec::Encode as _;
use jam_bootstrap_service_common::Instruction;
use jam_interface::JamChainSource;
use jam_rpc_interface::JamRpcInterface;
use jam_types::{AuthQueue, AuthorizerHash, CoreIndex, ServiceId};
use parachain_service_interface::{
	authorization::{Command, CONTROL_COMMAND_PREFIX},
	types::ParaId,
};

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
	/// The target para's authorizer: what gets written into the core being assigned.
	pub aura: Aura,
	/// The carrier core's authorizer: what the control package itself has to satisfy. A separate
	/// credential because the carrier is a different para, possibly with a different collator set
	/// on a different curve, and the token has to hash to what that core's queue actually holds.
	pub carrier: Aura,
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
///
/// It rides a core still holding the *genesis* authorizer and nothing else. Handing the privilege
/// over is service 0's to do, service 0's work items only get past the null authorizer, and
/// assignment to parasim is one-way: `free-core` parks a core under the AURA authorizer rather
/// than returning it to the null one, so no genesis core is ever made again. Grant every core
/// parasim is to own while one is still unassigned; once the last one goes, this command has no
/// lane left on that network.
pub async fn grant(jam: &JamRpcInterface, args: &Args, core: CoreIndex) -> Result<(), String> {
	let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash;
	match cores::assigner(jam, best, core).await? {
		current if current == args.service => {
			tracing::info!("core {core} is already assigned by service {current}");
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
		tracing::info!(
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
	tracing::info!("core {core} is now assigned by service {}", args.service);
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
	tracing::info!("core {core} → para {} (authorizer 0x{})", para.0, hex(&target.0));
	route(jam, args, core, target, Command::Assign { para_id: para, core, authorizer: target.0 })
		.await?;
	cores::report(jam, core, target).await
}

/// Park `core`: no para on it, but the same AURA authorizer, so commands still reach it.
///
/// The parked config is built from `--collators`/`--scheme` — normally the very values the core
/// was assigned with — because the hash has to travel with the command: accumulate cannot read an
/// authorizer config, not even its own package's, so it can no more derive a parked hash than an
/// assigned one.
pub async fn free(jam: &JamRpcInterface, args: &Args, core: CoreIndex) -> Result<(), String> {
	let target = args.aura.parked_hash();
	tracing::info!("core {core} → parked (authorizer 0x{})", hex(&target.0));
	route(jam, args, core, target, Command::Free { core, parked_authorizer: target.0 }).await?;
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
			tracing::info!("core {core} is still assigned by the bootstrap service; going through it");
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

/// Submit a package whose only job is to carry `command`.
///
/// The command *is* the work item's payload: accumulate, which is where `assign` has to be
/// called, sees neither the package nor its authorization, so refine reads the command and hands
/// it on in its work output. The token is `sudo`, which is what gets the package past an
/// authorizer that has no para to match the item against.
async fn control_package(
	jam: &JamRpcInterface,
	args: &Args,
	target_core: CoreIndex,
	command: Command,
) -> Result<(), String> {
	let core = args.via_core.unwrap_or(target_core);
	let anchor = Anchor::fetch(jam, args.service).await?;
	let head = cores::queue_head(jam, anchor.context.anchor, core).await?;

	// The carrier core has to be under the very authorizer this package names, or JAM refuses it
	// before parasim ever sees it — and a refused package is one more thing that looks like a
	// silent failure. A *parked* carrier is as good as an assigned one: parking keeps the AURA
	// authorizer in place, which is the whole reason the recovery dance is gone.
	let parked = head == args.carrier.parked_hash();
	let authorizer = if parked {
		args.carrier.parked_authorizer()
	} else {
		args.carrier.authorizer(args.via_para)
	};
	if authorizer.hash(jam_std_common::hash_raw) != head {
		return Err(carrier_mismatch(args, core, head));
	}
	tracing::info!(
		"carrying the command on core {core}, under {}",
		if parked {
			"the parked authorizer".to_string()
		} else {
			format!("para {}'s authorizer", args.via_para.0)
		}
	);

	let mut payload = CONTROL_COMMAND_PREFIX.to_vec();
	command.encode_to(&mut payload);
	let mut package = anchor.package(authorizer, vec![anchor.item(args.service, payload)]);
	package.authorization = args.carrier.token(&package, true)?;
	submit_and_follow(jam, core, &package).await
}

/// Both hashes the carrier credential can produce, so the operator can see which one the core was
/// expected to hold.
fn carrier_mismatch(args: &Args, core: CoreIndex, head: AuthorizerHash) -> String {
	format!(
		"core {core} holds authorizer 0x{}, but a token for para {} hashes to 0x{} and a parked \
		 one to 0x{}. Name the para that core is actually running with --via-para, or a core that \
		 is running one with --via-core; if that para has its own collator set or curve, name \
		 those too with --via-collators/--via-scheme.",
		hex(&head.0),
		args.via_para.0,
		hex(&args.carrier.hash(args.via_para).0),
		hex(&args.carrier.parked_hash().0),
	)
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
