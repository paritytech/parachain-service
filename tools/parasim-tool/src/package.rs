//! Building, submitting and following a work package.
//!
//! Every subcommand that puts something on chain — a mock parachain block, a bootstrap
//! instruction, a core-assignment command — needs the same three things: a refine context around
//! a usable anchor, a package wrapped around one work item, and a follow loop that waits for JAM
//! to report it. They live here so the commands differ only in what they actually say.

use std::time::Duration;

use futures::StreamExt as _;

use cumulus_jam_interface::{
	JamChainSource, JamStateSource, JamWorkPackageSubmission, WorkPackageStatus,
};
use cumulus_jam_rpc_interface::JamRpcInterface;
use jam_types::{
	Authorization, Authorizer, CodeHash, RefineContext, ServiceId, WorkItem, WorkPackage,
	WorkPackageHash, WorkPayload,
};

/// How long to follow a package before giving up on it.
const FOLLOW_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for a service's code to reach the lookup anchor.
const CODE_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Gap between code-availability checks; finality moves once per slot at best.
const CODE_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Substrate hash length.
pub const HASH_LEN: usize = 32;

/// The refine context every package in one run shares, and the gas the chain allows.
pub struct Anchor {
	pub context: RefineContext,
	pub code_hash: CodeHash,
	refine_gas_limit: u64,
	accumulate_gas_limit: u64,
}

impl Anchor {
	/// Anchor at the current best block, with a lookup anchor at which `service`'s code is
	/// already available.
	pub async fn fetch(jam: &JamRpcInterface, service: ServiceId) -> Result<Self, String> {
		let anchor = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let jam_std_common::VersionedParameters::V1(parameters) =
			jam.parameters().await.map_err(|e| format!("parameters: {e}"))?;
		let info = jam
			.service_info(anchor.header_hash, service)
			.await
			.map_err(|e| format!("service info: {e}"))?
			.ok_or_else(|| format!("service {service} is not registered"))?;

		let finalized = wait_for_code(jam, service, *info.code_hash).await?;
		let context = RefineContext {
			anchor: anchor.header_hash,
			state_root: jam
				.state_root(anchor.header_hash)
				.await
				.map_err(|e| format!("state root: {e}"))?,
			beefy_root: jam
				.beefy_root(anchor.header_hash)
				.await
				.map_err(|e| format!("beefy root: {e}"))?,
			lookup_anchor: finalized.header_hash,
			lookup_anchor_slot: finalized.slot,
			prerequisites: Default::default(),
		};
		Ok(Self {
			context,
			code_hash: CodeHash::from(*info.code_hash),
			refine_gas_limit: parameters.max_refine_gas,
			accumulate_gas_limit: parameters.max_accumulate_gas,
		})
	}

	/// One work item for `service`, with the whole package's gas allowance.
	pub fn item(&self, service: ServiceId, payload: Vec<u8>) -> WorkItem {
		WorkItem {
			service,
			code_hash: self.code_hash,
			payload: WorkPayload(payload),
			refine_gas_limit: self.refine_gas_limit,
			accumulate_gas_limit: self.accumulate_gas_limit,
			import_segments: Default::default(),
			extrinsics: Default::default(),
			// Nothing this tool submits exports anything. A count that does not match what
			// refine produces is answered with `BadExports`, whichever way it differs.
			export_count: 0,
		}
	}

	/// Wrap items in a package under `authorizer`, initially with no authorization token.
	///
	/// The token is left empty because an AURA collator signs the package *without* it: the
	/// signable hash excludes the token, which is what lets the signature live inside it.
	pub fn package(&self, authorizer: Authorizer, items: Vec<WorkItem>) -> WorkPackage {
		WorkPackage {
			authorization: Authorization::default(),
			// Validators fetch the authorizer code by preimage lookup from this service, and
			// both the null authorizer and ours are hosted by the bootstrap service.
			auth_code_host: 0,
			authorizer,
			context: self.context.clone(),
			items: items.try_into().expect("callers build one item; qed"),
		}
	}
}

/// The hash JAM identifies a work package by: blake2b-256 of its jam-codec encoding, which is
/// what `jam_std_common::build_encoded_bundle` puts at the front of a bundle.
pub fn work_package_hash(package: &WorkPackage) -> WorkPackageHash {
	jam_std_common::hash_raw(&jam_codec::Encode::encode(package)).into()
}

/// Submit the package and print each status until JAM reports it.
///
/// Returning at `Reported` rather than at accumulation is deliberate: for a chain of packages it
/// is what keeps them pipelined, and for a control package the caller has a better completion
/// signal to wait on — the state the command was meant to change.
pub async fn submit_and_follow(
	jam: &JamRpcInterface,
	core: u16,
	package: &WorkPackage,
) -> Result<(), String> {
	let package_hash = work_package_hash(package);
	jam.submit_work_package(core, package, Vec::new())
		.await
		.map_err(|e| format!("submitting the work package: {e}"))?;
	tracing::info!("submitted {package_hash:?} to core {core}");

	let mut statuses = jam
		.work_package_status_stream(package_hash, package.context.anchor, false)
		.await
		.map_err(|e| format!("following the work package: {e}"))?;

	let follow = async {
		while let Some(status) = statuses.next().await {
			tracing::info!("  status: {status:?}");
			match status {
				// Neither status says the package *succeeded*: a report is produced whether
				// refine returned a value or an error, and `Ready` only means "queued for
				// accumulation". The caller decides the outcome by watching state.
				WorkPackageStatus::Reported { .. } | WorkPackageStatus::Ready { .. } => {
					tracing::info!("  reported on chain");
					return Ok(());
				},
				WorkPackageStatus::Failed(reason) =>
					return Err(format!("the work package failed: {reason}")),
				WorkPackageStatus::Reportable { .. } => {},
			}
		}
		Err("the status stream closed before the package was reported".to_string())
	};

	match tokio::time::timeout(FOLLOW_TIMEOUT, follow).await {
		Ok(result) => result,
		Err(_) => Err(format!("gave up after {FOLLOW_TIMEOUT:?}")),
	}
}

/// Wait until the service's code is available at a finalized block, and return that block for
/// use as the package's `lookup_anchor`.
///
/// JAM fetches service code as of the `lookup_anchor`, so a package naming an anchor from before
/// the code was provided fails with `BadCode` and refine never runs — with nothing logged by the
/// service, which makes it look as though the service was never invoked. Finality lags the head
/// by a couple of slots, so submitting straight after `create-service` hits this every time and
/// then mysteriously starts working. Waiting here makes a cold deploy behave like a warm one.
async fn wait_for_code(
	jam: &JamRpcInterface,
	service: ServiceId,
	code_hash: [u8; HASH_LEN],
) -> Result<cumulus_jam_interface::BlockDesc, String> {
	use jam_std_common::Node as _;

	let deadline = tokio::time::Instant::now() + CODE_WAIT_TIMEOUT;
	loop {
		let finalized = jam.finalized_block().await.map_err(|e| format!("finalized block: {e}"))?;
		let len = jam
			.node()
			.service_preimage_len(finalized.header_hash, service, code_hash)
			.await
			.map_err(|e| format!("looking up the service code: {e}"))?;
		if let Some(len) = len {
			tracing::info!("service {service} code ({len} bytes) is available at the lookup anchor");
			return Ok(finalized);
		}
		if tokio::time::Instant::now() >= deadline {
			return Err(format!(
				"service {service} code is still unavailable at the finalized block after \
				 {CODE_WAIT_TIMEOUT:?}; was the service created?"
			));
		}
		tracing::info!("waiting for service {service} code to be available at the lookup anchor...");
		tokio::time::sleep(CODE_POLL_INTERVAL).await;
	}
}
