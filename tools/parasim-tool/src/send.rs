//! `send`: submit one or more linked mock parachain work packages to parasim.
//!
//! This stands in for a real collator so that parasim's refine/accumulate path can be exercised by
//! hand. It does the parts of collation that matter for that: pick an anchor, fetch a state proof
//! of the para's current head at that anchor, build a block that chains onto it, submit the
//! package, and follow it until the stored head moves.
//!
//! The proof is the point. parasim refuses any package that cannot prove what the para's previous
//! head was, so there is no offline payload it will accept.
//!
//! With `--chain N` the run submits N packages in one go, each building on the block the previous
//! one carried. Only the first can prove its parent from JAM state; the rest name a parent that is
//! still in flight — refined but not accumulated — and nothing but accumulate's reorder buffer
//! puts them back in order. That is the pipelining case, and it is only testable from a single
//! invocation, because the packages have to overlap in flight.
//!
//! Every package rides the para's AURA authorizer, signed as whichever dev collator the lookup
//! anchor's slot names. There is no unauthorized lane left: parasim reads the para id from the
//! authorizer config and refuses a package that has none.

use std::time::Duration;

use codec::Encode as _;

use crate::{
	aura::Aura,
	cores,
	format::hex,
	package::{submit_and_follow, Anchor},
};
use jam_interface::{JamChainSource, JamStateSource, StorageKey};
use jam_rpc_interface::JamRpcInterface;
use jam_state_helpers::StateProof;
use jam_types::{ServiceId, WorkPackage};
use parachain_service_interface::{
	candidate::ParachainCandidate,
	types::{ParaId, ValidationCodeHash},
};

/// Proof sizes are bounded by the trie depth, so this only has to be comfortably large.
const PROOF_SIZE_LIMIT: u32 = 1024 * 1024;
/// How long to wait for accumulate to store the new head after the last package is reported.
const HEAD_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Extra head-wait budget per package, since a chain accumulates one link at a time.
const HEAD_WAIT_PER_PACKAGE: Duration = Duration::from_secs(6);
/// Gap between head checks.
const HEAD_POLL_INTERVAL: Duration = Duration::from_secs(3);
/// Substrate hash length.
const HASH_LEN: usize = 32;

/// What `send` needs to know.
pub struct Args {
	pub service: ServiceId,
	pub para: ParaId,
	pub core: u16,
	pub chain: usize,
	pub tamper: Option<Tamper>,
	pub tamper_at: usize,
	pub aura: Aura,
}

/// A deliberate defect to plant in one package of the chain, so that the rejection it should draw
/// can be watched for. `Proof` is refused by refine; the other two are accepted there and dealt
/// with by accumulate, which is the only authority on lineage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Tamper {
	/// Corrupt the anchor state proof.
	Proof,
	/// Name a parent no package in the run ever built, so accumulate has nothing to apply the
	/// block onto. This is the only way to park a buffer entry on demand.
	WrongParent,
	/// Build a sibling of the package before it rather than its child: refine accepts it, because
	/// at the anchor its parent really is the accumulated head, but by the time it accumulates
	/// that head has moved on.
	Stale,
}

impl Tamper {
	/// What this defect should draw, for the operator to compare against the node log.
	fn expected_rejection(self) -> &'static str {
		match self {
			Tamper::Proof => "refine: InvalidProof",
			Tamper::WrongParent => "accumulate: a buffer park, for a parent that never comes",
			Tamper::Stale => "accumulate: a stale-head drop (refine accepts the package)",
		}
	}
}

/// A parent that no package in this run ever built, so naming it can only be a mistake or a lie.
fn forged_parent_hash() -> [u8; HASH_LEN] {
	jam_state_helpers::blake2_256(b"parasim-tool: a parent nobody built")
}

/// The package built for one position in the chain, as far as the next one needs to know it.
struct Link {
	header: Vec<u8>,
	number: u32,
}

pub async fn run(jam: &JamRpcInterface, args: &Args) -> Result<(), String> {
	if args.chain == 0 {
		return Err("--chain must be at least 1".to_string());
	}
	if args.tamper.is_some() && args.tamper_at >= args.chain {
		return Err(format!(
			"--tamper-at {} is past the end of a chain of {}",
			args.tamper_at, args.chain
		));
	}
	if args.tamper == Some(Tamper::WrongParent) && args.tamper_at == 0 {
		return Err("--tamper wrong-parent needs an earlier package to be parked behind: a para \
		            with no stored head accepts its first block whatever parent it names, so use \
		            --tamper-at 1 or more"
			.to_string());
	}
	if args.tamper == Some(Tamper::Stale) && args.tamper_at == 0 {
		return Err(
			"--tamper stale needs an earlier package to be superseded by: use --tamper-at 1 or more"
				.to_string(),
		);
	}

	// One anchor for the whole chain, chosen at build time, with the proof fetched at that same
	// anchor: the state root parasim sees in its refine context must be the one the proof was
	// built against. Holding it fixed across the chain is also what makes the run meaningful —
	// every package after the first proves a head that predates its own parent, so accumulate is
	// the only place its lineage can be settled.
	let anchor = Anchor::fetch(jam, args.service).await?;
	let anchor_state_root = *anchor.context.state_root;
	println!("anchor {:?} state root {:?}", anchor.context.anchor, anchor.context.state_root);

	// The core has to be running this para, or the package is refused before parasim sees it.
	let authorizer = args.aura.hash(args.para);
	let head = cores::queue_head(jam, anchor.context.anchor, args.core).await?;
	if head != authorizer {
		return Err(format!(
			"core {} holds authorizer 0x{}, but para {} hashes to 0x{}; assign the core first \
			 (or check --collators and --slot-duration)",
			args.core,
			hex(&head.0),
			args.para.0,
			hex(&authorizer.0),
		));
	}

	let service_local_key = parasim_service::para_head_key(args.para);
	let state_key =
		jam_state_helpers::service_value_state_key(args.service, &service_local_key);
	let proof = jam
		.state_proof(
			anchor.context.anchor,
			StorageKey(state_key),
			StorageKey(state_key),
			PROOF_SIZE_LIMIT,
		)
		.await
		.map_err(|e| format!("state proof: {e}"))?;
	let proof = to_state_proof(&proof);

	// Verify locally with the very code parasim runs, so a rejection on-chain means the chain
	// disagreed about the state — not that this tool built something malformed.
	let stored = jam_state_helpers::verify(&proof, &anchor_state_root, &state_key)
		.map_err(|e| format!("the node's own proof does not verify: {e:?}"))?;
	let accumulated = stored.as_deref().map(decode_para_info).transpose()?;

	match &accumulated {
		Some(head) => println!(
			"para {} head is {} bytes at number {}",
			args.para.0,
			head.len(),
			block_number(head)?
		),
		None => println!("para {} has no head yet; the chain starts at its first block", args.para.0),
	}

	let mut link: Option<Link> = None;
	// The head the para should end at: whatever the last package that is *expected to succeed*
	// carried. The tampered package never applies, and its successors name its head as their
	// parent — a head that never lands — so the whole tail falls with it.
	let mut expected_head = accumulated.clone();

	for index in 0..args.chain {
		let tamper = args.tamper.filter(|_| index == args.tamper_at);
		let doomed = args.tamper.is_some() && index >= args.tamper_at;
		println!();
		println!("=== package {} of {} ===", index + 1, args.chain);

		// A stale package deliberately ignores the block in flight and builds on the accumulated
		// head again, so it is a sibling of its predecessor rather than its child.
		let parent_link = link.as_ref().filter(|_| tamper != Some(Tamper::Stale));
		let (parent_hash, number) = match parent_link {
			Some(link) => (jam_state_helpers::blake2_256(&link.header), link.number + 1),
			None => match &accumulated {
				// A substrate header's hash is the blake2b-256 of its encoding, which is what
				// the next block must name as its parent.
				Some(head) => (jam_state_helpers::blake2_256(head), block_number(head)? + 1),
				None => ([0u8; HASH_LEN], 0),
			},
		};
		let parent_hash = match tamper {
			Some(Tamper::WrongParent) => forged_parent_hash(),
			_ => parent_hash,
		};

		let mut proof = proof.clone();
		if tamper == Some(Tamper::Proof) {
			// Corrupt a node so the proof no longer matches the anchor state root.
			let node = proof.nodes.first_mut().ok_or("nothing to tamper with")?;
			node[HASH_LEN] ^= 0xff;
		}
		if let Some(tamper) = tamper {
			println!("tampering: {tamper:?}; expect {}", tamper.expected_rejection());
		}

		// The nonce keeps two blocks built on the same parent — a stale sibling and the package it
		// was meant to extend — from being the same block, and so the same work package.
		let header = header_bytes(parent_hash, number, args.para, index as u32);
		let payload = build_payload(&anchor_state_root, &proof, &header);
		println!("block number {number} parent 0x{}", hex(&parent_hash));
		let package = build_package(&anchor, args, payload)?;
		submit_and_follow(jam, args.core, &package).await?;

		if !doomed {
			expected_head = Some(header.clone());
		}
		link = Some(Link { header, number });
	}

	// A `Reported` status only means JAM put the work *report* on chain, and a report is produced
	// whether refine returned a head or an error. So the real outcome is whether the stored head
	// changed, and that takes another slot or two while accumulate runs.
	println!();
	let timeout = HEAD_WAIT_TIMEOUT + HEAD_WAIT_PER_PACKAGE * args.chain as u32;
	let observed =
		wait_for_head(jam, args.service, &service_local_key, expected_head.as_deref(), timeout)
			.await?;
	report(&accumulated, expected_head.as_deref(), observed.as_deref())
}

/// Say whether the run ended where it should have.
fn report(
	before: &Option<Vec<u8>>,
	expected: Option<&[u8]>,
	observed: Option<&[u8]>,
) -> Result<(), String> {
	if observed != expected {
		return Err(format!(
			"the para head is {}, expected {} (see the node log, or `display-inflight`)",
			describe_head(observed),
			describe_head(expected),
		));
	}
	match expected {
		Some(head) if Some(head) != before.as_deref() =>
			println!("para head advanced to {} bytes: {}", head.len(), hex(head)),
		_ => println!("para head unchanged, as expected: the package never applied"),
	}
	Ok(())
}

fn describe_head(head: Option<&[u8]>) -> String {
	match head {
		Some(head) => format!("{} bytes ({})", head.len(), hex(head)),
		None => "unset".to_string(),
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
	anchor_state_root: &[u8; HASH_LEN],
	proof: &StateProof,
	header: &[u8],
) -> Vec<u8> {
	let pov = v3_pov(header, &(*anchor_state_root, proof.clone()).encode());
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
	pov.extend_from_slice(&header_bytes([0u8; HASH_LEN], 0, ParaId(0), 0));
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
/// The nonce goes in `extrinsics_root`, for the same reason: two blocks on the same parent have to
/// differ somewhere or they are the same block.
fn header_bytes(parent_hash: [u8; HASH_LEN], number: u32, para: ParaId, nonce: u32) -> Vec<u8> {
	let mut header = parent_hash.to_vec();
	compact(number, &mut header);
	let mut state_root = [0u8; HASH_LEN];
	state_root[..4].copy_from_slice(&para.0.to_le_bytes());
	header.extend_from_slice(&state_root);
	let mut extrinsics_root = [0u8; HASH_LEN];
	extrinsics_root[..4].copy_from_slice(&nonce.to_le_bytes());
	header.extend_from_slice(&extrinsics_root);
	header.push(0); // empty Digest
	header
}

fn compact(value: u32, out: &mut Vec<u8>) {
	codec::Compact(value).encode_to(out);
}

/// Wrap one payload in a single-item package under the para's AURA authorizer, signed as the
/// collator the lookup anchor names.
fn build_package(
	anchor: &Anchor,
	args: &Args,
	payload: Vec<u8>,
) -> Result<WorkPackage, String> {
	let mut package = anchor
		.package(args.aura.authorizer(args.para), vec![anchor.item(args.service, payload)]);
	package.authorization = args.aura.token(&package, None)?;
	Ok(package)
}

/// Wait for the para head to reach `expected`, returning the last head observed.
///
/// Nothing announces that accumulate has run: `Reported` fires a slot or two earlier, and there is
/// no "accumulated" work-package status. Polling the stored value is the only signal, and it is
/// also the signal a real collator follows.
async fn wait_for_head(
	jam: &JamRpcInterface,
	service: ServiceId,
	service_local_key: &[u8],
	expected: Option<&[u8]>,
	timeout: Duration,
) -> Result<Option<Vec<u8>>, String> {
	let deadline = tokio::time::Instant::now() + timeout;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let stored = jam
			.service_value(best.header_hash, service, service_local_key)
			.await
			.map_err(|e| format!("reading the para head back: {e}"))?;
		let head = stored.as_deref().map(decode_para_info).transpose()?;
		if head.as_deref() == expected || tokio::time::Instant::now() >= deadline {
			return Ok(head);
		}
		tokio::time::sleep(HEAD_POLL_INTERVAL).await;
	}
}
