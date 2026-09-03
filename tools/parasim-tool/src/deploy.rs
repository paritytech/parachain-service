//! `deploy-authorizer`: get the AURA authorizer blob hosted by the bootstrap service.
//!
//! Validators fetch authorizer code by preimage lookup from the service named in a package's
//! `auth_code_host`, so a hash nobody hosts is a core nobody can use. Hosting takes two steps
//! that cannot be merged: the service must *request* the preimage before anyone may provide it,
//! and the request only exists once the soliciting package has accumulated.

use std::{path::Path, time::Duration};

use cumulus_jam_interface::JamChainSource;
use cumulus_jam_rpc_interface::JamRpcInterface;
use jam_bootstrap_service_common::Instruction;
use jam_std_common::Node as _;
use jam_types::ToAny as _;

use crate::{bootstrap, cores::BOOTSTRAP_SERVICE, format::hex};

/// How long to wait for the solicit to accumulate, and then for the preimage to be integrated.
const WAIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Gap between checks; neither step can advance more than once per block.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

pub async fn run(jam: &JamRpcInterface, blob: &Path) -> Result<(), String> {
	let blob = std::fs::read(blob).map_err(|e| format!("reading {}: {e}", blob.display()))?;
	let hash = jam_std_common::hash_raw(&blob);
	let len = blob.len() as u64;
	tracing::info!("authorizer code hash 0x{} ({len} bytes)", hex(&hash));

	if available_at_finalized(jam, hash).await?.is_some() {
		tracing::info!("already available at the lookup anchor; nothing to do");
		return Ok(());
	}

	tracing::info!("soliciting it into service {BOOTSTRAP_SERVICE}");
	bootstrap::instruct(jam, None, vec![Instruction::Solicit { hash: hash.into_any(), len }])
		.await?;
	wait_until("the solicit to accumulate", || async {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let request = jam
			.node()
			.service_request(best.header_hash, BOOTSTRAP_SERVICE, hash, len as u32)
			.await
			.map_err(|e| format!("reading the preimage request: {e}"))?;
		Ok(request.is_some())
	})
	.await?;

	tracing::info!("providing the blob");
	jam.node()
		.submit_preimage(BOOTSTRAP_SERVICE, blob.into())
		.await
		.map_err(|e| format!("providing the preimage: {e}"))?;
	wait_until("the preimage to be available at a finalized block", || async {
		Ok(available_at_finalized(jam, hash).await?.is_some())
	})
	.await?;

	tracing::info!("authorizer 0x{} is hosted by service {BOOTSTRAP_SERVICE}", hex(&hash));
	Ok(())
}

/// The blob's length if it is already a preimage of the bootstrap service at a finalized block.
///
/// Finalized, not best: that is the block a work package may name as its lookup anchor, and it is
/// where a validator will look the code up.
async fn available_at_finalized(
	jam: &JamRpcInterface,
	hash: [u8; 32],
) -> Result<Option<u32>, String> {
	let finalized = jam.finalized_block().await.map_err(|e| format!("finalized block: {e}"))?;
	jam.node()
		.service_preimage_len(finalized.header_hash, BOOTSTRAP_SERVICE, hash)
		.await
		.map_err(|e| format!("looking up the authorizer code: {e}"))
}

async fn wait_until<F, Fut>(what: &str, mut done: F) -> Result<(), String>
where
	F: FnMut() -> Fut,
	Fut: std::future::Future<Output = Result<bool, String>>,
{
	let deadline = tokio::time::Instant::now() + WAIT_TIMEOUT;
	loop {
		if done().await? {
			return Ok(());
		}
		if tokio::time::Instant::now() >= deadline {
			return Err(format!("gave up waiting for {what} after {WAIT_TIMEOUT:?}"));
		}
		tracing::info!("waiting for {what}...");
		tokio::time::sleep(POLL_INTERVAL).await;
	}
}
