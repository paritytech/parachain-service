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
//! one carried. Only the first can prove its parent from JAM state: the rest have parents that are
//! still in flight — refined but not accumulated — so their headers travel as imported segments.
//! That is the pipelining case, and it is only testable from a single invocation, because the
//! parent's `wp_hash` has to be linked while it is still in flight.

use std::time::Duration;

use futures::StreamExt as _;

use codec::Encode as _;

use crate::{bundle, format::hex};
use jam_interface::{
	JamChainSource, JamStateSource, JamWorkPackageSubmission, StorageKey, WorkPackageStatus,
};
use jam_rpc_interface::JamRpcInterface;
use jam_state_helpers::StateProof;
use jam_std_common::ImportData;
use jam_types::{
	Authorization, Authorizer, CodeHash, ImportSpec, RefineContext, RootIdentifier, ServiceId,
	WorkItem, WorkPackage, WorkPackageHash, WorkPayload,
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
}

/// A deliberate defect to plant in one package of the chain, so that the rejection it should draw
/// can be watched for. Every kind but `Proof` attacks the import path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Tamper {
	/// Corrupt the anchor state proof.
	Proof,
	/// Build on a parent that is neither the accumulated head nor anything imported.
	NoImport,
	/// Import the true parent header, but name a different parent in the block.
	WrongParent,
	/// Import a zero-segment — what a parent whose own refine failed exports.
	EmptyImport,
	/// Import a segment that is not a decodable header.
	GarbageImport,
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
			Tamper::NoImport => "refine: MissingImport",
			Tamper::WrongParent => "refine: ParentHashMismatch",
			Tamper::EmptyImport => "refine: EmptyImportedHeader",
			Tamper::GarbageImport => "refine: UndecodableImportedHeader",
			Tamper::Stale => "accumulate: a stale-head drop (refine accepts the package)",
		}
	}

	/// Whether refine itself refuses the package. A refused item exports nothing, so its
	/// successors import zero-segments; a package refine accepts still exports its head even if
	/// accumulate later drops it.
	fn fails_refine(self) -> bool {
		self != Tamper::Stale
	}
}

/// A parent that no package in this run ever built, so naming it can only be a mistake or a lie.
fn forged_parent_hash() -> [u8; HASH_LEN] {
	jam_state_helpers::blake2_256(b"parasim-tool: a parent nobody built")
}

/// The bytes a `garbage-import` plants where a header should be.
const GARBAGE_HEADER: &[u8] = b"not a header";

/// The package built for one position in the chain, as far as the next one needs to know it.
struct Link {
	wp_hash: WorkPackageHash,
	header: Vec<u8>,
	number: u32,
	/// Whether this package's refine is expected to fail. JAM replaces a failed item's exports
	/// with zero-segments, so the next package imports zeroes rather than this header.
	refine_failed: bool,
}

/// What a package imports, and how the chain is asked to authenticate it.
enum Import {
	/// Nothing: the block must build directly on the head proven from JAM state.
	None,
	/// Segment 0 of the parent package, named by the parent's work-package hash. The chain
	/// resolves that hash to the parent's export root and validates the mapping, which is what
	/// makes an imported header worth trusting.
	Parent { wp_hash: WorkPackageHash, segment: Vec<u8> },
	/// A segment of our own making, committed to by a root we compute ourselves. A `Direct` root
	/// is an unauthenticated claim by the submitter, so this is how a forged parent header would
	/// really reach refine — which is the case the empty/undecodable-header guard exists for.
	Forged { segment: Vec<u8> },
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
		return Err(
			"--tamper wrong-parent needs a parent package to import: use --tamper-at 1 or more"
				.to_string(),
		);
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
	// every package after the first proves a head that predates its own parent, so its parent can
	// only come from the import.
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
	let proof = to_state_proof(&proof);

	// Verify locally with the very code parasim runs, so a rejection on-chain means the chain
	// disagreed about the state — not that this tool built something malformed.
	let stored = jam_state_helpers::verify(&proof, &anchor_state_root, &state_key)
		.map_err(|e| format!("the node's own proof does not verify: {e:?}"))?;
	let accumulated = stored.as_deref().map(decode_para_info).transpose()?;

	if args.tamper == Some(Tamper::NoImport) && accumulated.is_none() {
		return Err("--tamper no-import needs a head to already be stored: parasim accepts an \
		            unparented first block, so there would be nothing to reject"
			.to_string());
	}

	match &accumulated {
		Some(head) => println!(
			"para {} head is {} bytes at number {}",
			args.para.0,
			head.len(),
			block_number(head)?
		),
		None => println!("para {} has no head yet; the chain starts at its first block", args.para.0),
	}

	let template = Template::fetch(jam, args, anchor).await?;
	let mut link: Option<Link> = None;
	// The head the para should end at: whatever the last package that is *expected to succeed*
	// carried. Packages from the tampered one onwards fail, and a failed package exports nothing
	// its successors can chain onto, so the whole tail falls with it.
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
		let honest_import = match parent_link {
			None => Import::None,
			Some(parent) => Import::Parent {
				wp_hash: parent.wp_hash,
				segment: if parent.refine_failed {
					bundle::zero_segment()
				} else {
					bundle::head_segment(&parent.header)
				},
			},
		};
		let (parent_hash, import) = plan_link(tamper, parent_hash, honest_import);

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
		let (specs, imports, prerequisites) = import_fields(import);
		println!("block number {number} parent 0x{} imports {}", hex(&parent_hash), specs.len());
		let package = template.package(args, payload, specs, prerequisites)?;
		let (wp_hash, encoded) = bundle::build(&package, imports);
		// A package with no imports needs no bundle, and submitting it the plain way keeps the
		// pre-pipelining path in use as well.
		let inline = (package.items[0].import_segments.len() > 0).then_some(encoded);
		submit_and_follow(jam, args.core, &package, wp_hash, inline).await?;

		if !doomed {
			expected_head = Some(header.clone());
		}
		let refine_failed = args.tamper.is_some_and(Tamper::fails_refine) && doomed;
		link = Some(Link { wp_hash, header, number, refine_failed });
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

/// Apply a tamper kind to the parent a package names and the segment it imports.
fn plan_link(
	tamper: Option<Tamper>,
	parent_hash: [u8; HASH_LEN],
	honest: Import,
) -> ([u8; HASH_LEN], Import) {
	match tamper {
		None | Some(Tamper::Proof) | Some(Tamper::Stale) => (parent_hash, honest),
		Some(Tamper::NoImport) => (forged_parent_hash(), Import::None),
		Some(Tamper::WrongParent) => (forged_parent_hash(), honest),
		// The parent hashes below are the ones the forged segment *would* hash to, so the block
		// passes the parent-hash check and the header guard is the only thing left to stop it.
		// `blake2(<empty>)` in particular is a public constant: without the guard, a zero-segment
		// exported by a failed package would be a parent anyone could name.
		Some(Tamper::EmptyImport) =>
			(jam_state_helpers::blake2_256(&[]), Import::Forged { segment: bundle::zero_segment() }),
		Some(Tamper::GarbageImport) => (
			jam_state_helpers::blake2_256(GARBAGE_HEADER),
			Import::Forged { segment: bundle::head_segment(GARBAGE_HEADER) },
		),
	}
}

/// The work item's import specs, the bundle's inline import data, and the package's prerequisites.
fn import_fields(import: Import) -> (Vec<ImportSpec>, Vec<ImportData>, Vec<WorkPackageHash>) {
	match import {
		Import::None => (Vec::new(), Vec::new(), Vec::new()),
		Import::Parent { wp_hash, segment } => {
			let (data, _) = bundle::import_data(segment);
			// The prerequisite is what orders accumulation; the import is what carries the parent
			// header. Both name the same package, and together they cost 2 of the 8 dependencies.
			(
				vec![ImportSpec { root: RootIdentifier::Indirect(wp_hash), index: 0 }],
				vec![data],
				vec![wp_hash],
			)
		},
		Import::Forged { segment } => {
			let (data, root) = bundle::import_data(segment);
			(
				vec![ImportSpec { root: RootIdentifier::Direct(root), index: 0 }],
				vec![data],
				Vec::new(),
			)
		},
	}
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
		_ => println!("para head unchanged, as expected: refine rejected the package"),
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

/// Everything a package in the chain shares: the anchor's context and the service's code hash.
///
/// Fetched once, because every package in a run is anchored at the same block.
struct Template {
	code_hash: CodeHash,
	context: RefineContext,
	refine_gas_limit: u64,
	accumulate_gas_limit: u64,
}

impl Template {
	async fn fetch(
		jam: &JamRpcInterface,
		args: &Args,
		anchor: jam_interface::BlockDesc,
	) -> Result<Self, String> {
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
		Ok(Self {
			code_hash: CodeHash::from(*service.code_hash),
			context,
			refine_gas_limit: parameters.max_refine_gas,
			accumulate_gas_limit: parameters.max_accumulate_gas,
		})
	}

	/// Wrap one payload in a single-item work package.
	fn package(
		&self,
		args: &Args,
		payload: Vec<u8>,
		import_segments: Vec<ImportSpec>,
		prerequisites: Vec<WorkPackageHash>,
	) -> Result<WorkPackage, String> {
		let item = WorkItem {
			service: args.service,
			code_hash: self.code_hash,
			payload: WorkPayload(payload),
			refine_gas_limit: self.refine_gas_limit,
			accumulate_gas_limit: self.accumulate_gas_limit,
			import_segments: import_segments
				.try_into()
				.map_err(|_| "too many import segments")?,
			extrinsics: Default::default(),
			// parasim always exports its new head on success. Declaring 0 would make even a
			// successful refine over-export, which JAM answers with `BadExports`.
			export_count: 1,
		};
		let mut context = self.context.clone();
		context.prerequisites = prerequisites.into();
		Ok(WorkPackage {
			authorization: Authorization::default(),
			auth_code_host: 0,
			// The dev genesis seeds every core's queue with the null authorizer and an empty
			// config.
			authorizer: Authorizer {
				code_hash: jam_null_authorizer_bin::HASH.into(),
				config: Default::default(),
			},
			context,
			items: vec![item].try_into().expect("one work item always fits; qed"),
		})
	}
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
///
/// Returning at `Reported` rather than at accumulation is what keeps a chain pipelined: the parent
/// is on chain as a report — which is what lets a guarantor resolve the child's `Indirect` import
/// — while its head is still nowhere in state, so the child really must import it.
async fn submit_and_follow(
	jam: &JamRpcInterface,
	core: u16,
	package: &WorkPackage,
	package_hash: WorkPackageHash,
	bundle: Option<Vec<u8>>,
) -> Result<(), String> {
	match bundle {
		Some(bundle) => {
			let size = bundle.len();
			jam.submit_bundle(core, bundle)
				.await
				.map_err(|e| format!("submitting the bundle: {e}"))?;
			println!("submitted bundle {package_hash:?} ({size} bytes) to core {core}");
		},
		None => {
			jam.submit_work_package(core, package, Vec::new())
				.await
				.map_err(|e| format!("submitting the work package: {e}"))?;
			println!("submitted {package_hash:?} to core {core}");
		},
	}

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
					println!("  reported on chain");
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
