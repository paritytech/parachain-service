//! `display-inflight`: work packages JAM has reported but not yet accumulated.
//!
//! Between "Reported" and the para head moving, a package is parked in JAM state: first in the
//! per-core availability assignments, then in the accumulation queue. Both hold the whole work
//! report, and a report carries refine's *output bytes* rather than a hash of them — so what a
//! package produced is readable over plain RPC, where it previously took the node's stdout to see.
//! A parasim *rejection* shows up here only as `BadExports`: refine exports its new head after its
//! checks pass, so a rejected item is one export short and JAM replaces its output. The reason
//! stays in the guarantor's log.
//!
//! Two codecs meet here. The state values are jam-codec (decoded by `jam-interface`); parasim's
//! own work output nested inside them is parity-scale-codec, because that is what parasim encodes
//! it with.

use std::{collections::BTreeMap, time::Duration};

use codec::DecodeAll as _;
use jam_interface::{
	HeaderHash, JamChainSource, JamStateSource, ServiceId, Slot, VersionedParameters, WorkReport,
};
use jam_rpc_interface::JamRpcInterface;
use jam_types::WorkDigest;
use parasim_service::{ParasimRefineError, ParasimWorkOutput};

use crate::{
	format::{hex, parse_header_hash},
	header,
};

/// How many hex digits of a hash to show; enough to recognise one, short enough to stay in line.
const HASH_DIGITS: usize = 8;

/// What `display-inflight` needs to know.
pub struct Args {
	pub service: ServiceId,
	pub para: Option<u32>,
	pub block: Option<String>,
	pub watch: bool,
	pub raw: bool,
}

/// A row's identity across samples: the same work item, seen at the same stage.
type RowKey = (&'static str, [u8; 32], usize);

const AVAILABLE: &str = "available";
const READY: &str = "ready";

pub async fn run(jam: &JamRpcInterface, args: &Args) -> Result<(), String> {
	let VersionedParameters::V1(parameters) =
		jam.parameters().await.map_err(|e| format!("parameters: {e}"))?;
	if args.watch {
		return watch(jam, args, parameters.epoch_period, parameters.slot_period_sec).await;
	}

	let at = match &args.block {
		Some(hash) => parse_header_hash(hash)?,
		None => jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash,
	};
	print_header(false);
	for row in sample(jam, args, at, parameters.epoch_period).await?.values() {
		println!("{row}");
	}
	Ok(())
}

/// Poll once per slot and report only what changed.
///
/// A one-shot read almost always finds nothing: a package is in flight for a couple of slots, so
/// hitting that window by hand takes luck. Watching alongside `send` in another terminal is what
/// this subcommand is really for.
async fn watch(
	jam: &JamRpcInterface,
	args: &Args,
	epoch_period: Slot,
	slot_period_sec: u16,
) -> Result<(), String> {
	print_header(true);
	let mut previous: BTreeMap<RowKey, String> = BTreeMap::new();
	let mut seen: Option<HeaderHash> = None;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		if seen != Some(best.header_hash) {
			seen = Some(best.header_hash);
			let current = sample(jam, args, best.header_hash, epoch_period).await?;
			for (key, row) in &current {
				if !previous.contains_key(key) {
					println!("+ {row}");
				}
			}
			for (key, row) in &previous {
				if !current.contains_key(key) {
					println!("- {row}");
				}
			}
			previous = current;
		}
		tokio::time::sleep(Duration::from_secs(slot_period_sec.into())).await;
	}
}

fn print_header(watching: bool) {
	let indent = if watching { "  " } else { "" };
	println!("{indent}{:<10}{:<6}{:<10}{:<9}{}", "stage", "core", "slot", "service", "outcome");
}

/// Every in-flight work digest at `at`, keyed so two samples can be compared.
async fn sample(
	jam: &JamRpcInterface,
	args: &Args,
	at: HeaderHash,
	epoch_period: Slot,
) -> Result<BTreeMap<RowKey, String>, String> {
	let mut rows = BTreeMap::new();

	let availability = jam.availability(at).await.map_err(|e| format!("availability: {e}"))?;
	for assignment in availability.iter().flatten() {
		collect(&mut rows, args, AVAILABLE, &assignment.report, assignment.report_slot, 0);
	}

	// The queue is indexed by epoch phase, so dating its entries needs the block's own slot.
	let now = jam.current_time(at).await.map_err(|e| format!("current time: {e}"))?;
	let ready = jam.ready_queue(at).await.map_err(|e| format!("ready queue: {e}"))?;
	for (phase, records) in ready.iter().enumerate() {
		let slot = phase_slot(phase as Slot, now, epoch_period);
		for record in records {
			collect(&mut rows, args, READY, &record.report, slot, record.deps.len());
		}
	}
	Ok(rows)
}

/// The absolute slot an accumulation-queue entry was queued at.
///
/// The queue holds one epoch of entries indexed by `slot % epoch_period`, so a phase names exactly
/// one slot in the epoch ending at `now`. Before the chain is an epoch old some phases name no
/// slot at all; those saturate to genesis rather than wrapping.
fn phase_slot(phase: Slot, now: Slot, epoch_period: Slot) -> Slot {
	now.saturating_sub((now + epoch_period - phase) % epoch_period)
}

fn collect(
	rows: &mut BTreeMap<RowKey, String>,
	args: &Args,
	stage: &'static str,
	report: &WorkReport,
	slot: Slot,
	waiting_on: usize,
) {
	for (index, digest) in report.results.iter().enumerate() {
		let Some(outcome) = describe(args, digest) else { continue };
		// A non-empty dependency set is *why* an entry is still queued, so say so.
		let waiting =
			if waiting_on == 0 { String::new() } else { format!(", waiting on {waiting_on}") };
		let row = format!(
			"{stage:<10}{:<6}{slot:<10}{:<9}{outcome}{waiting}",
			report.core_index, digest.service,
		);
		rows.insert((stage, *report.package_spec.hash, index), row);
	}
}

/// The `outcome` column for one work digest, or `None` if `--para` rules the row out.
fn describe(args: &Args, digest: &WorkDigest) -> Option<String> {
	let output = match &digest.result {
		// A `WorkError` means the item produced no value at all. Since parasim exports its new
		// head only after its checks pass, every parasim rejection lands here as `BadExports`:
		// the item declared one export and made none, so JAM discards its output. Which check
		// refused it is only in the guarantor's log.
		Err(error) => return Some(format!("REFINE FAILED: {error:?}")),
		Ok(output) => &output.0,
	};

	let refined =
		if digest.service == args.service { decode_output(output) } else { Refined::Foreign };
	if !matches_para(args.para, &refined) {
		return None;
	}
	Some(if args.raw { format!("0x{}", hex(output)) } else { render(&refined, output.len()) })
}

/// What parasim's refine put in the work output.
enum Refined {
	Head(ParasimWorkOutput),
	Rejected(ParasimRefineError),
	/// parasim's output, but nothing this tool's version of parasim understands.
	Unknown,
	/// Some other service's output.
	Foreign,
}

/// Decode parasim's work output, which is parity-scale-codec even though the report carrying it is
/// jam-codec.
///
/// A rejection refine can still report — one that leaves the export count intact — arrives as a
/// *successful* work output holding the reason where the head would have gone. Telling the two
/// apart by trying each is unambiguous, because a `ParasimWorkOutput` carries a 32-byte parent
/// hash and a `ParasimRefineError` is exactly one byte.
fn decode_output(bytes: &[u8]) -> Refined {
	if let Ok(output) = ParasimWorkOutput::decode_all(&mut &bytes[..]) {
		return Refined::Head(output);
	}
	match ParasimRefineError::decode_all(&mut &bytes[..]) {
		Ok(error) => Refined::Rejected(error),
		Err(_) => Refined::Unknown,
	}
}

/// Whether a `--para` filter admits this result.
///
/// Only a decoded head names a para. A rejection does not, and filtering it out would hide the one
/// row the filter's user is most likely waiting for, so anything unattributable is kept.
fn matches_para(para: Option<u32>, refined: &Refined) -> bool {
	match (para, refined) {
		(None, _) => true,
		(Some(wanted), Refined::Head(output)) => output.para_id.0 == wanted,
		(Some(_), Refined::Foreign) => false,
		(Some(_), Refined::Rejected(_) | Refined::Unknown) => true,
	}
}

fn render(refined: &Refined, len: usize) -> String {
	match refined {
		Refined::Head(output) => {
			let head = &output.head_data[..];
			match header::decode(head) {
				Ok(header) => format!(
					"head {} bytes, number {}, parent {}…",
					head.len(),
					header.number,
					&hex(&header.parent_hash)[..HASH_DIGITS],
				),
				Err(error) => format!("head {} bytes, undecodable header: {error}", head.len()),
			}
		},
		Refined::Rejected(error) => format!("REJECTED: {error:?}"),
		Refined::Unknown => format!("{len} bytes, not a parasim result"),
		Refined::Foreign => format!("{len} bytes"),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::Encode as _;

	#[test]
	fn output_and_rejection_are_told_apart_works() {
		// The whole feature rests on this: a rejected package is an `Ok` work output holding an
		// error, so the two encodings must not be confusable in either direction.
		let head = ParasimWorkOutput {
			para_id: parachain_service_interface::types::ParaId(7),
			head_data: vec![1u8; 40].try_into().expect("40 bytes fit; qed"),
			parent_head_hash: [2u8; 32],
		};
		assert!(matches!(decode_output(&head.encode()), Refined::Head(decoded) if decoded == head));
		assert!(matches!(
			decode_output(&ParasimRefineError::InvalidProof.encode()),
			Refined::Rejected(ParasimRefineError::InvalidProof)
		));
		assert!(matches!(decode_output(&[0xff, 0xff]), Refined::Unknown));
	}

	#[test]
	fn phase_slot_dates_the_current_epoch_works() {
		// Phase 3 of an epoch of 12 that is currently at slot 100 (phase 4) was one slot ago; the
		// phase just ahead of `now` belongs to the previous epoch.
		assert_eq!(phase_slot(3, 100, 12), 99);
		assert_eq!(phase_slot(4, 100, 12), 100);
		assert_eq!(phase_slot(5, 100, 12), 89);
		// A phase that predates genesis has no slot to name.
		assert_eq!(phase_slot(5, 2, 12), 0);
	}

	#[test]
	fn para_filter_keeps_unattributable_rows_works() {
		let head = Refined::Head(ParasimWorkOutput {
			para_id: parachain_service_interface::types::ParaId(0),
			head_data: Default::default(),
			parent_head_hash: [0u8; 32],
		});
		assert!(matches_para(Some(0), &head));
		assert!(!matches_para(Some(1), &head));
		assert!(matches_para(Some(1), &Refined::Rejected(ParasimRefineError::InvalidProof)));
		assert!(!matches_para(Some(1), &Refined::Foreign));
	}
}
