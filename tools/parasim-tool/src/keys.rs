//! `display-key`: read a service-storage entry and decode it.
//!
//! Two subjects are understood: the para head (`parahead`) and the reorder buffer (`buffer`).
//! Naming the subject leaves room for other entries as the service grows, and it is also a
//! reminder that the bytes are not self-describing: the caller has to say what they expect.

use std::time::Duration;

use codec::Decode as _;
use cumulus_jam_interface::{
	HeaderHash, JamChainSource, JamStateSource, ServiceId, VersionedParameters,
};
use cumulus_jam_rpc_interface::JamRpcInterface;
use parachain_service_interface::types::ParaId;
use parasim_service::buffer::{BufferedCandidate, StoredHead, BUFFER_CAP};

use crate::{
	format::{hex, parse_header_hash},
	header,
};

/// Which of a para's service-storage entries to read.
#[derive(Clone, Copy)]
pub enum Subject {
	/// The para's head, as stored in the service's `parachains` map.
	Parahead,
	/// The heads accumulate has parked until their parent arrives.
	Buffer,
}

impl Subject {
	fn key(self, para: ParaId) -> Vec<u8> {
		match self {
			Self::Parahead => parasim_service::para_head_key(para),
			Self::Buffer => parasim_service::buffer_key(para),
		}
	}
}

/// What `display-key` needs to know.
pub struct Args {
	pub service: ServiceId,
	pub para: ParaId,
	pub subject: Subject,
	pub block: Option<String>,
	pub watch: bool,
	pub raw: bool,
}

pub async fn run(jam: &JamRpcInterface, args: &Args) -> Result<(), String> {
	if args.watch {
		return watch(jam, args).await;
	}
	let at = resolve_block(jam, args.block.clone()).await?;
	show(jam, args, at).await
}

/// Poll once per slot and re-print the entry whenever its bytes change.
///
/// Same convention as `display-inflight --watch`: one verb, `--watch` streams changes. Comparing
/// the stored bytes rather than the block means a slot that did not touch this para prints
/// nothing, which is what makes it usable next to a `send` in another terminal.
async fn watch(jam: &JamRpcInterface, args: &Args) -> Result<(), String> {
	let VersionedParameters::V1(parameters) =
		jam.parameters().await.map_err(|e| format!("parameters: {e}"))?;
	let key = args.subject.key(args.para);
	let mut previous: Option<Option<Vec<u8>>> = None;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let stored = jam
			.service_value(best.header_hash, args.service, &key)
			.await
			.map_err(|e| format!("reading the entry: {e}"))?;
		if previous.as_ref() != Some(&stored) {
			tracing::info!("para {} changed at block 0x{}", args.para.0, hex(&*best.header_hash));
			show(jam, args, best.header_hash).await?;
			println!();
			previous = Some(stored);
		}
		tokio::time::sleep(Duration::from_secs(parameters.slot_period_sec.into())).await;
	}
}

/// Print the entry as it stands at `at`, decoded unless `raw`.
async fn show(jam: &JamRpcInterface, args: &Args, at: HeaderHash) -> Result<(), String> {
	let service_local_key = args.subject.key(args.para);
	print_location(at, args.service, args.para, &service_local_key);

	let stored = jam
		.service_value(at, args.service, &service_local_key)
		.await
		.map_err(|e| format!("reading the entry: {e}"))?;

	match (args.subject, stored) {
		(Subject::Parahead, None) => {
			println!("\nno entry: para {} has no head at this block", args.para.0);
			Ok(())
		},
		(Subject::Buffer, None) => {
			println!("\nbuffer is empty: nothing parked for para {} at this block", args.para.0);
			Ok(())
		},
		(subject, Some(stored)) => {
			let label = match subject {
				Subject::Parahead => "ParaInfo     ",
				Subject::Buffer => "ReorderBuffer",
			};
			println!("\n{label} {} bytes", stored.len());
			if args.raw {
				println!("0x{}", hex(&stored));
				return Ok(());
			}
			match subject {
				Subject::Parahead => print_para_info(&stored),
				// The stored head is read alongside the buffer, because a parked head only means
				// something against the head it is waiting for: the same entry is next in line or
				// already orphaned depending on where the head has got to.
				Subject::Buffer => {
					let head = jam
						.service_value(at, args.service, &parasim_service::para_head_key(args.para))
						.await
						.map_err(|e| format!("reading the para head: {e}"))?;
					print_buffer(&stored, StoredHead::read(head.as_deref()))
				},
			}
		},
	}
}

/// Decode and print a stored `ParaInfo`, then the substrate header inside its `head_data`.
fn print_para_info(stored: &[u8]) -> Result<(), String> {
	let info = parasim_service::ParaInfoLite::decode(&mut &stored[..])
		.map_err(|e| format!("not a decodable ParaInfo: {e} (try --raw)"))?;
	let head = info.head_data.into_inner();

	println!("  head_data           {} bytes", head.len());
	println!("  validation_code     {:?}", info.validation_code);
	println!("  pending_upgrade     {:?}", info.pending_upgrade);
	println!("  total_state_balance {}", info.total_state_balance);
	println!("  used_state_balance  {}", info.used_state_balance);
	println!("  is_deregistering    {}", info.is_deregistering);

	println!("\nhead (substrate header)");
	println!("  hash        0x{}", hex(&jam_state_helpers::blake2_256(&head)));
	match header::decode(&head) {
		Ok(header) => {
			println!("  parent_hash 0x{}", hex(&header.parent_hash));
			println!("  number      {}", header.number);
			println!("  state_root  0x{}", hex(&header.state_root));
		},
		Err(error) => println!("  (undecodable: {error})"),
	}
	println!("  encoded     0x{}", hex(&head));
	Ok(())
}

/// Decode and print the parked heads, each against the head the para is at.
fn print_buffer(stored: &[u8], head: StoredHead) -> Result<(), String> {
	let entries = Vec::<BufferedCandidate>::decode(&mut &stored[..])
		.map_err(|e| format!("not a decodable reorder buffer: {e} (try --raw)"))?;

	match head {
		StoredHead::Empty => println!("stored head none: this para has no head yet"),
		StoredHead::Unreadable => println!(
			"stored head unreadable, so accumulate can judge neither lineage nor height (see \
			 display-key parahead)"
		),
		StoredHead::At { hash, number } =>
			println!("stored head number {number}, hash 0x{}", hex(&hash)),
	}
	println!("depth {}/{BUFFER_CAP}", entries.len());
	if entries.is_empty() {
		println!("\nbuffer is empty, though accumulate removes the entry when it drains");
		return Ok(());
	}

	for (index, entry) in entries.iter().enumerate() {
		println!("\nentry {index}");
		println!("  number       {}", entry.number);
		println!("  head         0x{}", hex(&entry.head_hash()));
		println!("  parent_head  0x{}", hex(&entry.parent_head_hash));
		println!("  arrived_slot {}", entry.arrived_slot);
		println!("  head_data    {} bytes", entry.head_data.len());
		println!("  status       {}", describe(classify(&entries, index, head)));
		if let Some(disagreement) = header_disagreement(entry) {
			println!("  WARNING      {disagreement}");
		}
	}
	Ok(())
}

/// Where a parked head stands relative to the stored head and to the other parked heads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
	/// Its parent is the stored head, so the next accumulate for this para applies it.
	///
	/// Accumulate drains in the same invocation that applies the parent, so this should not be
	/// visible in stored state at all: seeing it means a drain stopped early, which is what a
	/// failed head write does.
	DrainsNext,
	/// Its parent is another parked head, so it drains behind that one.
	ChainsOnto(usize),
	/// The stored head has reached its number, so its parent can never be the stored head again.
	Overtaken,
	/// Neither the stored head nor anything parked is its parent.
	Waiting,
}

/// Classify entry `index`, mirroring the rules `parasim_service::buffer` applies on arrival.
fn classify(entries: &[BufferedCandidate], index: usize, head: StoredHead) -> Status {
	let entry = &entries[index];
	if let StoredHead::At { hash, number } = head {
		if entry.parent_head_hash == hash {
			return Status::DrainsNext;
		}
		if entry.number <= number {
			return Status::Overtaken;
		}
	}
	match entries.iter().position(|parked| parked.head_hash() == entry.parent_head_hash) {
		Some(parent) => Status::ChainsOnto(parent),
		None => Status::Waiting,
	}
}

fn describe(status: Status) -> String {
	match status {
		Status::DrainsNext => "drains next: its parent is the stored head".into(),
		Status::ChainsOnto(parent) =>
			format!("waiting on entry {parent}, which is parked here too"),
		Status::Overtaken =>
			"orphaned: the stored head has reached this number, so the next accumulate for this \
			 para evicts it"
				.into(),
		Status::Waiting =>
			"waiting: its parent is neither the stored head nor parked here, so it needs a report \
			 that has not arrived"
				.into(),
	}
}

/// Whether the header inside `head_data` says something other than the entry does.
///
/// The two are written by different parts of refine's output, so a disagreement is a bug rather
/// than a state a debugger should read past.
fn header_disagreement(entry: &BufferedCandidate) -> Option<String> {
	match header::decode(&entry.head_data) {
		Err(error) => Some(format!("head_data is not a decodable header: {error}")),
		Ok(header) if header.number != entry.number =>
			Some(format!("head_data's header says number {}", header.number)),
		Ok(header) if header.parent_hash != entry.parent_head_hash =>
			Some(format!("head_data's header says parent 0x{}", hex(&header.parent_hash))),
		Ok(_) => None,
	}
}

/// The block a read happens at: the one named, or the current best.
async fn resolve_block(jam: &JamRpcInterface, at: Option<String>) -> Result<HeaderHash, String> {
	match at {
		Some(hash) => parse_header_hash(&hash),
		None => Ok(jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash),
	}
}

/// Print where a read is coming from, including both forms of the key.
///
/// Both, because which of the two an RPC wants is easy to get wrong.
fn print_location(at: HeaderHash, service: ServiceId, para: ParaId, service_local_key: &[u8]) {
	let state_key = jam_state_helpers::service_value_state_key(service, service_local_key);
	println!("block       0x{}", hex(&*at));
	println!("service     {service}");
	println!("para        {}", para.0);
	println!(
		"service key 0x{}  (this is what set_storage/serviceValue take)",
		hex(service_local_key)
	);
	println!("state key   0x{}  (this is what stateProof/stateValue take)", hex(&state_key));
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::Encode as _;

	/// A candidate carrying a real substrate header, so `head_hash` and `header_disagreement`
	/// see the bytes accumulate would see.
	fn candidate(parent: [u8; 32], number: u32) -> BufferedCandidate {
		let mut head = parent.to_vec();
		codec::Compact(number).encode_to(&mut head);
		head.extend_from_slice(&[0u8; 64]);
		head.push(0);
		BufferedCandidate {
			parent_head_hash: parent,
			head_data: head.try_into().expect("a header fits; qed"),
			number,
			arrived_slot: 0,
		}
	}

	#[test]
	fn entries_are_classified_against_the_stored_head_works() {
		let root = candidate([1u8; 32], 1);
		let head = StoredHead::At { hash: root.head_hash(), number: 1 };
		// Two parked heads chaining onto the stored head: the first drains next, the second
		// behind it. Saying only "parked" for both would hide that the queue is in fact ordered.
		let child = candidate(root.head_hash(), 2);
		let grandchild = candidate(child.head_hash(), 3);
		let entries = vec![child, grandchild];
		assert_eq!(classify(&entries, 0, head), Status::DrainsNext);
		assert_eq!(classify(&entries, 1, head), Status::ChainsOnto(0));
	}

	#[test]
	fn an_entry_the_head_has_reached_reads_as_orphaned_works() {
		// A losing fork's child: it still chains onto its parked parent, but the stored head is
		// past its number, so the next accumulate evicts both. `Overtaken` has to win over
		// `ChainsOnto`, or the buffer would read as if the pair were still going to drain.
		let parent = candidate([2u8; 32], 4);
		let child = candidate(parent.head_hash(), 5);
		let entries = vec![parent, child];
		let head = StoredHead::At { hash: [9u8; 32], number: 5 };
		assert_eq!(classify(&entries, 0, head), Status::Overtaken);
		assert_eq!(classify(&entries, 1, head), Status::Overtaken);
	}

	#[test]
	fn an_unknown_parent_reads_as_waiting_works() {
		let entries = vec![candidate([3u8; 32], 7)];
		assert_eq!(
			classify(&entries, 0, StoredHead::At { hash: [9u8; 32], number: 6 }),
			Status::Waiting
		);
		// With no head stored there is no number to be past and no hash to match, so nothing
		// can be claimed about height or lineage.
		assert_eq!(classify(&entries, 0, StoredHead::Empty), Status::Waiting);
		assert_eq!(classify(&entries, 0, StoredHead::Unreadable), Status::Waiting);
	}

	#[test]
	fn head_data_disagreeing_with_the_entry_is_flagged_works() {
		// The entry's fields and its head bytes come from different parts of refine's output, so
		// they can only disagree through a bug — and a debugger reading past that would draw the
		// wrong conclusion about why a head is stuck.
		let honest = candidate([4u8; 32], 11);
		assert_eq!(header_disagreement(&honest), None);

		let mut lying_number = honest.clone();
		lying_number.number = 12;
		assert!(header_disagreement(&lying_number)
			.expect("number disagrees")
			.contains("number 11"));

		let mut lying_parent = honest.clone();
		lying_parent.parent_head_hash = [5u8; 32];
		assert!(header_disagreement(&lying_parent)
			.expect("parent disagrees")
			.contains("parent 0x0404"));

		let truncated = BufferedCandidate {
			head_data: vec![0u8; 8].try_into().expect("8 bytes fit; qed"),
			..honest
		};
		assert!(header_disagreement(&truncated)
			.expect("not a header")
			.contains("not a decodable header"));
	}
}
