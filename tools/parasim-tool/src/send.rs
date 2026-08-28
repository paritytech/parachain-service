//! `send`: submit a mock parachain work package to parasim.
//!
//! This stands in for a real collator so that parasim's refine/accumulate path can be exercised by
//! hand. It does the parts of collation that matter for that: pick an anchor, fetch a state proof
//! of the para's current head at that anchor, build a block that chains onto it, submit the
//! package, and follow it until the stored head moves.
//!
//! The proof is the point. parasim refuses any package that cannot prove what the para's previous
//! head was, so there is no offline payload it will accept.

use std::time::Duration;

use futures::StreamExt as _;

use codec::Encode as _;

use crate::format::hex;
use jam_interface::{
	JamChainSource, JamStateSource, JamWorkPackageSubmission, StorageKey, WorkPackageStatus,
};
use jam_rpc_interface::JamRpcInterface;
use jam_state_helpers::StateProof;
use jam_types::{
	Authorization, Authorizer, CodeHash, RefineContext, ServiceId, WorkItem, WorkPackage,
	WorkPayload,
};
use parachain_service_interface::{
	candidate::ParachainCandidate,
	types::{ParaId, ValidationCodeHash},
};

/// Proof sizes are bounded by the trie depth, so this only has to be comfortably large.
const PROOF_SIZE_LIMIT: u32 = 1024 * 1024;
/// How long to follow a package before giving up on it.
const FOLLOW_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for a freshly created service's code to reach the lookup anchor.
const CODE_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Gap between code-availability checks; finality moves once per slot at best.
const CODE_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// How long to wait for accumulate to store the new head after the package is reported.
const HEAD_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Gap between head checks.
const HEAD_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Substrate hash length.
const HASH_LEN: usize = 32;

/// What `send` needs to know.
pub struct Args {
	pub service: ServiceId,
	pub para: ParaId,
	pub core: u16,
	pub tamper: bool,
}

pub async fn run(jam: &JamRpcInterface, args: &Args) -> Result<(), String> {
	// The anchor is chosen at build time and the proof is fetched at that same anchor, so that the
	// state root parasim sees in its refine context is the one the proof was built against.
	let anchor = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
	let anchor_state_root =
		jam.state_root(anchor.header_hash).await.map_err(|e| format!("state root: {e}"))?;
	println!("anchor {:?} state root {:?}", anchor.header_hash, anchor_state_root);

	let service_local_key = parasim_service::para_head_key(args.para);
	let state_key =
		jam_state_helpers::service_value_state_key(args.service, &service_local_key);
	let proof = jam
		.state_proof(
			anchor.header_hash,
			StorageKey(state_key),
			StorageKey(state_key),
			PROOF_SIZE_LIMIT,
		)
		.await
		.map_err(|e| format!("state proof: {e}"))?;
	let mut proof = to_state_proof(&proof);

	// Verify locally with the very code parasim runs, so a rejection on-chain means the chain
	// disagreed about the state — not that this tool built something malformed.
	let stored = jam_state_helpers::verify(&proof, &anchor_state_root, &state_key)
		.map_err(|e| format!("the node's own proof does not verify: {e:?}"))?;

	let (parent_hash, number) = match &stored {
		Some(stored) => {
			let head = decode_para_info(stored)?;
			// A substrate header's hash is the blake2b-256 of its encoding, which is what the next
			// block must name as its parent.
			let parent = jam_state_helpers::blake2_256(&head);
			let number = block_number(&head)? + 1;
			println!(
				"para {} head is {} bytes at number {}; building on {:?}",
				args.para.0,
				head.len(),
				number - 1,
				parent,
			);
			(parent, number)
		},
		None => {
			println!("para {} has no head yet; building its first block", args.para.0);
			([0u8; HASH_LEN], 0)
		},
	};

	if args.tamper {
		// Corrupt a node so the proof no longer matches the anchor state root. parasim must reject
		// the package and leave the para head untouched.
		let node = proof.nodes.first_mut().ok_or("nothing to tamper with")?;
		node[HASH_LEN] ^= 0xff;
		println!("tampered with the proof: refine is expected to reject this package");
	}

	let payload = build_payload(args.para, &anchor_state_root, &proof, parent_hash, number);
	let package = build_package(jam, args, anchor, payload).await?;
	submit_and_follow(jam, args.core, &package).await?;

	// A `Reported` status only means JAM put the work *report* on chain, and a report is produced
	// whether refine returned a head or an error. So the real outcome is whether the stored head
	// changed, and that takes another slot or two while accumulate runs.
	let moved = wait_for_head_change(jam, args.service, &service_local_key, &stored).await?;
	match (moved, args.tamper) {
		(Some(head), false) => {
			println!("para head advanced to {} bytes: {}", head.len(), hex(&head));
			Ok(())
		},
		(None, true) => {
			println!("para head unchanged, as expected: refine rejected the tampered proof");
			Ok(())
		},
		(None, false) =>
			Err("the para head did not change; refine rejected the package (see the node log)"
				.to_string()),
		(Some(_), true) =>
			Err("the para head advanced despite a tampered proof: verification is not working"
				.to_string()),
	}
}

/// Convert the node's JSON-shaped proof into the SCALE-encodable one that travels in the PoV.
///
/// polkajam's `RangeProof` has no SCALE codec at all — it is a host-side, base64/JSON type — so
/// the wire form parasim reads is deliberately our own.
fn to_state_proof(proof: &jam_interface::RangeProof) -> StateProof {
	StateProof {
		nodes: proof.nodes.iter().map(|node| **node).collect(),
		values: proof.values.iter().map(|(key, value)| (**key, value.to_vec())).collect(),
	}
}

/// A header's block number.
///
/// Read out of the stored head rather than counted locally, so repeated runs keep producing
/// distinct blocks instead of rebuilding the same one forever.
fn block_number(head: &[u8]) -> Result<u32, String> {
	use codec::{Compact, Decode as _};
	// `Header` = parent_hash(32) ++ compact number ++ ...
	let mut rest = head.get(HASH_LEN..).ok_or("stored head is too short to be a header")?;
	let Compact(number) = Compact::<u32>::decode(&mut rest)
		.map_err(|e| format!("stored head has no block number: {e}"))?;
	Ok(number)
}

/// Pull `head_data` out of a stored `ParaInfo`, which begins with it.
fn decode_para_info(stored: &[u8]) -> Result<Vec<u8>, String> {
	use codec::Decode as _;
	let info = parasim_service::ParaInfoLite::decode(&mut &stored[..])
		.map_err(|e| format!("stored ParaInfo does not decode: {e}"))?;
	Ok(info.head_data.into_inner())
}

/// Build the work-item payload: a `ParachainCandidate` whose PoV is a V3 `ParachainBlockData`
/// carrying one fake block and the anchor state proof.
fn build_payload(
	para: ParaId,
	anchor_state_root: &[u8; HASH_LEN],
	proof: &StateProof,
	parent_hash: [u8; HASH_LEN],
	number: u32,
) -> Vec<u8> {
	let pov = v3_pov(
		&header_bytes(parent_hash, number, para),
		&(*anchor_state_root, proof.clone()).encode(),
	);
	ParachainCandidate { validation_code_hash: ValidationCodeHash([0u8; HASH_LEN]), pov }.encode()
}

/// A V3 `ParachainBlockData`: one block, an empty `CompactProof`, an empty `SchedulingProof`, and
/// one `additional_data` slot holding the anchor state proof.
fn v3_pov(header: &[u8], anchor_state_proof: &[u8]) -> Vec<u8> {
	let mut pov = b"VERSIONEDPBD".to_vec();
	pov.push(3);

	// Vec<Block>: one block with no extrinsics.
	compact(1, &mut pov);
	pov.extend_from_slice(header);
	compact(0, &mut pov);

	// CompactProof { encoded_nodes: Vec<Vec<u8>> }: parasim runs no PVF, so there is nothing to
	// witness about the parachain's own state.
	compact(0, &mut pov);

	// An empty SchedulingProof, matching upstream's `Default`: JAM has no relay-chain scheduling,
	// and upstream documents this as how to carry additional data without one.
	compact(0, &mut pov); // header_chain: empty
	pov.extend_from_slice(&header_bytes([0u8; HASH_LEN], 0, ParaId(0)));
	pov.push(0); // signed_scheduling_info: None

	// Vec<Option<AdditionalData>>: one slot, one entry.
	compact(1, &mut pov);
	pov.push(1); // Some
	compact(1, &mut pov);
	parasim_service::pov::ANCHOR_STATE_PROOF_KEY.as_bytes().to_vec().encode_to(&mut pov);
	anchor_state_proof.to_vec().encode_to(&mut pov);
	pov
}

/// Encode a substrate `Header<u32, BlakeTwo256>`.
///
/// The para id goes in `state_root` purely so that different paras produce different heads; a real
/// collator would put an actual state root there.
fn header_bytes(parent_hash: [u8; HASH_LEN], number: u32, para: ParaId) -> Vec<u8> {
	let mut header = parent_hash.to_vec();
	compact(number, &mut header);
	let mut state_root = [0u8; HASH_LEN];
	state_root[..4].copy_from_slice(&para.0.to_le_bytes());
	header.extend_from_slice(&state_root);
	header.extend_from_slice(&[0u8; HASH_LEN]); // extrinsics_root
	header.push(0); // empty Digest
	header
}

fn compact(value: u32, out: &mut Vec<u8>) {
	codec::Compact(value).encode_to(out);
}

/// Wrap the payload in a single-item work package anchored at `anchor`.
async fn build_package(
	jam: &JamRpcInterface,
	args: &Args,
	anchor: jam_interface::BlockDesc,
	payload: Vec<u8>,
) -> Result<WorkPackage, String> {
	let jam_std_common::VersionedParameters::V1(parameters) =
		jam.parameters().await.map_err(|e| format!("parameters: {e}"))?;
	let service = jam
		.service_info(anchor.header_hash, args.service)
		.await
		.map_err(|e| format!("service info: {e}"))?
		.ok_or_else(|| format!("service {} is not registered", args.service))?;

	let finalized = wait_for_code(jam, args.service, *service.code_hash).await?;
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

	let item = WorkItem {
		service: args.service,
		code_hash: CodeHash::from(*service.code_hash),
		payload: WorkPayload(payload),
		refine_gas_limit: parameters.max_refine_gas,
		accumulate_gas_limit: parameters.max_accumulate_gas,
		import_segments: Default::default(),
		extrinsics: Default::default(),
		export_count: 0,
	};
	Ok(WorkPackage {
		authorization: Authorization::default(),
		auth_code_host: 0,
		// The dev genesis seeds every core's queue with the null authorizer and an empty config.
		authorizer: Authorizer {
			code_hash: jam_null_authorizer_bin::HASH.into(),
			config: Default::default(),
		},
		context,
		items: vec![item].try_into().expect("one work item always fits; qed"),
	})
}

/// Wait for accumulate to store a head different from `before`, returning it, or `None` if it
/// never changed.
///
/// Nothing announces that accumulate has run: `Reported` fires a slot or two earlier, and there is
/// no "accumulated" work-package status. Polling the stored value is the only signal, and it is
/// also the signal a real collator follows.
async fn wait_for_head_change(
	jam: &JamRpcInterface,
	service: ServiceId,
	service_local_key: &[u8],
	before: &Option<Vec<u8>>,
) -> Result<Option<Vec<u8>>, String> {
	let deadline = tokio::time::Instant::now() + HEAD_WAIT_TIMEOUT;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let stored = jam
			.service_value(best.header_hash, service, service_local_key)
			.await
			.map_err(|e| format!("reading the para head back: {e}"))?;
		// `stored` holds a `ParaInfo`; comparing it whole is enough to see the head move.
		if let Some(stored) = &stored {
			if Some(stored) != before.as_ref() {
				let head = decode_para_info(stored)?;
				return Ok(Some(head));
			}
		}
		if tokio::time::Instant::now() >= deadline {
			return Ok(None);
		}
		tokio::time::sleep(HEAD_POLL_INTERVAL).await;
	}
}

/// Wait until the service's code is available at a finalized block, and return that block for
/// use as the package's `lookup_anchor`.
///
/// JAM fetches service code as of the `lookup_anchor`, so a package naming an anchor from before
/// the code was provided fails with `BadCode` and refine never runs — with nothing logged by the
/// service, which makes it look as though the service was never invoked. Finality lags the head by
/// a couple of slots, so submitting straight after `create-service` hits this every time and then
/// mysteriously starts working. Waiting here makes a cold deploy behave like a warm one.
async fn wait_for_code(
	jam: &JamRpcInterface,
	service: ServiceId,
	code_hash: [u8; HASH_LEN],
) -> Result<jam_interface::BlockDesc, String> {
	use jam_std_common::Node as _;

	let deadline = tokio::time::Instant::now() + CODE_WAIT_TIMEOUT;
	loop {
		let finalized =
			jam.finalized_block().await.map_err(|e| format!("finalized block: {e}"))?;
		let len = jam
			.node()
			.service_preimage_len(finalized.header_hash, service, code_hash)
			.await
			.map_err(|e| format!("looking up the service code: {e}"))?;
		if let Some(len) = len {
			println!("service {service} code ({len} bytes) is available at the lookup anchor");
			return Ok(finalized);
		}
		if tokio::time::Instant::now() >= deadline {
			return Err(format!(
				"service {service} code is still unavailable at the finalized block after \
				 {CODE_WAIT_TIMEOUT:?}; was the service created?"
			));
		}
		println!("waiting for service {service} code to be available at the lookup anchor...");
		tokio::time::sleep(CODE_POLL_INTERVAL).await;
	}
}

/// Submit the package and print each status until JAM reports it.
async fn submit_and_follow(
	jam: &JamRpcInterface,
	core: u16,
	package: &WorkPackage,
) -> Result<(), String> {
	// The package hash is over the *jam-codec* encoding, not SCALE.
	let package_hash = jam_types::WorkPackageHash::from(jam_state_helpers::blake2_256(
		&jam_codec::Encode::encode(package),
	));
	jam.submit_work_package(core, package, Vec::new())
		.await
		.map_err(|e| format!("submitting the work package: {e}"))?;
	println!("submitted {package_hash:?} to core {core}; following its status");

	let mut statuses = jam
		.work_package_status_stream(package_hash, package.context.anchor, false)
		.await
		.map_err(|e| format!("following the work package: {e}"))?;

	let follow = async {
		while let Some(status) = statuses.next().await {
			println!("  status: {status:?}");
			match status {
				// Neither status says the package *succeeded*: a report is produced whether refine
				// returned a head or an error, and `Ready` only means "queued for accumulation".
				// The caller decides the outcome by watching the head.
				WorkPackageStatus::Reported { .. } | WorkPackageStatus::Ready { .. } => {
					println!("  reported on chain; waiting to see whether the head moves");
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
