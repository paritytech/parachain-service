//! What JAM state says about a core: who may assign it, what its queue holds, and what has
//! reached its pool.
//!
//! A failed `assign` writes nothing to the chain — no log, no event, no changed value — so every
//! command that assigns reads the answer back from here rather than trusting that submitting was
//! enough.

use std::time::Duration;

use jam_codec::DecodeAll as _;
use jam_interface::{HeaderHash, JamChainSource, JamStateSource, StorageKey};
use jam_rpc_interface::JamRpcInterface;
use jam_std_common::{Privileges, SystemKey};
use jam_types::{AuthConfig, Authorizer, AuthorizerHash, CoreIndex, ServiceId};

use crate::format::hex;

/// The JAM bootstrap service: manager, registrar, designator and assigner of every core at
/// genesis, and the only route to a core parasim has not been handed yet.
pub const BOOTSTRAP_SERVICE: ServiceId = 0;

/// How long to wait for an assignment to show up in the queue.
const QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(60);
/// Gap between queue checks; a queue can only change once per block.
const QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// The queue entry that means "this core is unassigned": polkajam's null authorizer with an
/// empty config, which is what every core's queue holds at genesis and what `free-core` puts
/// back.
pub fn unassigned() -> AuthorizerHash {
	Authorizer { code_hash: jam_null_authorizer_bin::HASH.into(), config: AuthConfig::default() }
		.hash(jam_std_common::hash_raw)
}

/// The service that may call `assign` for `core`. Anything it is not, cannot.
pub async fn assigner(
	jam: &JamRpcInterface,
	at: HeaderHash,
	core: CoreIndex,
) -> Result<ServiceId, String> {
	let raw = jam
		.state_value(at, StorageKey::from(SystemKey::Privileges))
		.await
		.map_err(|e| format!("reading the privileges: {e}"))?
		.ok_or("the chain has no privileges entry")?;
	let privileges = Privileges::decode_all(&mut &raw[..])
		.map_err(|e| format!("the privileges do not decode: {e}"))?;
	privileges
		.assign
		.get(core as usize)
		.copied()
		.ok_or_else(|| format!("there is no core {core} on this chain"))
}

/// The authorizer at the head of `core`'s queue: what a package submitted there must satisfy.
pub async fn queue_head(
	jam: &JamRpcInterface,
	at: HeaderHash,
	core: CoreIndex,
) -> Result<AuthorizerHash, String> {
	let queues = jam.auth_queues(at).await.map_err(|e| format!("reading the queues: {e}"))?;
	let queue = queues
		.get(core as usize)
		.ok_or_else(|| format!("there is no core {core} on this chain"))?;
	queue.get(0).copied().ok_or_else(|| format!("core {core} has an empty queue"))
}

/// Everything in `core`'s pool, newest last: the authorizers a package may actually be reported
/// under right now, as opposed to the queue, which only says what will be refilled from.
pub async fn pool(
	jam: &JamRpcInterface,
	at: HeaderHash,
	core: CoreIndex,
) -> Result<Vec<AuthorizerHash>, String> {
	let pools = jam.auth_pools(at).await.map_err(|e| format!("reading the pools: {e}"))?;
	let pool = pools
		.get(core as usize)
		.ok_or_else(|| format!("there is no core {core} on this chain"))?;
	Ok(pool.to_vec())
}

/// Wait for `core`'s queue to hold `expected`, returning what it holds when the wait ends.
pub async fn wait_for_queue(
	jam: &JamRpcInterface,
	core: CoreIndex,
	expected: AuthorizerHash,
) -> Result<AuthorizerHash, String> {
	let deadline = tokio::time::Instant::now() + QUEUE_WAIT_TIMEOUT;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let head = queue_head(jam, best.header_hash, core).await?;
		if head == expected || tokio::time::Instant::now() >= deadline {
			return Ok(head);
		}
		tokio::time::sleep(QUEUE_POLL_INTERVAL).await;
	}
}

/// Report how `core` ended up, and fail loudly if the queue did not take the assignment.
///
/// Waits for the pool as well as the queue. A work package is only reportable under an
/// authorizer that is *in the pool*, and the pool refills from the queue one entry per block, so
/// a core whose queue has just changed still refuses everything for a block or two. Returning
/// before then would hand the caller a core that looks assigned and is not yet usable.
pub async fn report(
	jam: &JamRpcInterface,
	core: CoreIndex,
	expected: AuthorizerHash,
) -> Result<(), String> {
	let observed = wait_for_queue(jam, core, expected).await?;
	if observed != expected {
		return Err(format!(
			"core {core}'s queue still holds 0x{}, expected 0x{} — a rejected `assign` leaves no \
			 trace on chain, so check that the assigner privilege is where you think it is",
			hex(&observed.0),
			hex(&expected.0),
		));
	}
	println!("core {core} queue is 0x{}", hex(&expected.0));

	let deadline = tokio::time::Instant::now() + QUEUE_WAIT_TIMEOUT;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let pool = pool(jam, best.header_hash, core).await?;
		let converged = pool.iter().filter(|entry| **entry == expected).count();
		if converged > 0 {
			println!("core {core} pool holds {converged} of {} copies of it", pool.len());
			return Ok(());
		}
		if tokio::time::Instant::now() >= deadline {
			return Err(format!(
				"core {core}'s queue holds the new authorizer but its pool never did; nothing can \
				 be reported on that core"
			));
		}
		println!("waiting for core {core}'s pool to pick the authorizer up...");
		tokio::time::sleep(QUEUE_POLL_INTERVAL).await;
	}
}
