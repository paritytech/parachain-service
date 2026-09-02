//! `display-authorizers`: what each core's authorizer pool and queue actually hold.
//!
//! The two are not the same thing and confusing them wastes a lot of time. `assign` writes the
//! *queue*; the pool refills from it one entry per block, and a package is only reportable under
//! an authorizer that has reached the *pool*. So a core whose queue changed a moment ago still
//! refuses everything, and a core that was just freed keeps working until its pool drains.
//!
//! Hashes are shown short and runs are collapsed, because a queue is eighty copies of one hash in
//! this design and printing them would bury the one entry that moved.

use std::time::Duration;

use jam_interface::{
	AuthorizerHash, HeaderHash, JamChainSource, JamStateSource, VersionedParameters,
};
use jam_rpc_interface::JamRpcInterface;
use parachain_service_interface::types::ParaId;

use crate::{aura::Aura, cores, format::hex};

/// How many paras to try when naming a hash. A match is a preimage match, so a label is proof
/// rather than a guess; this only bounds how far the tool looks for one.
const LABELLED_PARAS: u32 = 32;

/// What `display-authorizers` needs to know.
pub struct Args {
	pub block: Option<String>,
	pub watch: bool,
	/// Every credential a hash on this chain could have been built from. None of them is trusted:
	/// a name is attached only where one of them reproduces a hash the chain holds, so a candidate
	/// that is wrong about the blob, the curve or the collator set names nothing.
	pub credentials: Vec<Aura>,
}

pub async fn run(jam: &JamRpcInterface, args: &Args) -> Result<(), String> {
	let names = names(&args.credentials);
	if args.watch {
		return watch(jam, &names).await;
	}

	let at = match &args.block {
		Some(hash) => crate::format::parse_header_hash(hash)?,
		None => jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash,
	};
	print!("{}", render(at, &sample(jam, at).await?, &names));
	Ok(())
}

/// Poll once per slot and re-print when anything moved.
///
/// Same convention as `display-inflight --watch`: one verb, `--watch` streams changes. A core
/// takes several blocks to hand over — the queue moves at once, the pool one entry at a time —
/// and watching is the only way to see that happen rather than infer it.
///
/// What is compared is the pools and queues themselves, the way `display-key --watch` compares
/// the stored bytes. The rendering is dated with the block it was read at, so diffing that would
/// report a change every block and drown the one block where something did move.
async fn watch(jam: &JamRpcInterface, names: &[(AuthorizerHash, String)]) -> Result<(), String> {
	let VersionedParameters::V1(parameters) =
		jam.parameters().await.map_err(|e| format!("parameters: {e}"))?;
	let mut previous: Option<Cores> = None;
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let current = sample(jam, best.header_hash).await?;
		if previous.as_ref() != Some(&current) {
			tracing::info!("authorizers changed at block 0x{}", hex(&*best.header_hash));
			print!("{}", render(best.header_hash, &current, names));
			previous = Some(current);
		}
		tokio::time::sleep(Duration::from_secs(parameters.slot_period_sec.into())).await;
	}
}

/// Every core's pool and queue, in core order: the state the display shows and the watch loop
/// compares.
type Cores = Vec<(Vec<AuthorizerHash>, Vec<AuthorizerHash>)>;

async fn sample(jam: &JamRpcInterface, at: HeaderHash) -> Result<Cores, String> {
	let pools = jam.auth_pools(at).await.map_err(|e| format!("reading the pools: {e}"))?;
	let queues = jam.auth_queues(at).await.map_err(|e| format!("reading the queues: {e}"))?;
	Ok(pools
		.iter()
		.zip(queues.iter())
		.map(|(pool, queue)| (pool.to_vec(), queue[..].to_vec()))
		.collect())
}

/// A sample as the display prints it, dated with the block it was read at.
fn render(at: HeaderHash, cores: &Cores, names: &[(AuthorizerHash, String)]) -> String {
	use std::fmt::Write as _;

	let mut out = String::new();
	let _ = writeln!(out, "block 0x{}", hex(&*at));
	for (core, (pool, queue)) in cores.iter().enumerate() {
		let _ = writeln!(out, "core {core}");
		let _ = writeln!(out, "  pool  {}", describe(pool, names, Some(&queue[..])));
		let _ = writeln!(out, "  queue {}", describe(queue, names, None));
	}
	out
}

/// One core's pool or queue as a line: runs of the same hash collapsed, each named where the tool
/// can reproduce it.
///
/// `refill` is the core's queue when `entries` is its pool. A pool entry the queue no longer holds
/// can never be put back, so it is what is left of an earlier assignment — kept alive only because
/// a core that reports every block takes its entry out before the refill would have evicted one.
/// Saying so is the difference between reading a stuck-looking pool as a bug and as history.
fn describe(
	entries: &[AuthorizerHash],
	names: &[(AuthorizerHash, String)],
	refill: Option<&[AuthorizerHash]>,
) -> String {
	if entries.is_empty() {
		return "(empty)".to_string();
	}
	runs(entries)
		.iter()
		.map(|(hash, count)| {
			let name = names.iter().find(|(known, _)| known == hash).map(|(_, name)| name.as_str());
			let stale = refill.is_some_and(|queue| !queue.contains(hash)).then_some("stale");
			let notes: Vec<&str> = name.into_iter().chain(stale).collect();
			let notes =
				if notes.is_empty() { String::new() } else { format!(" ({})", notes.join(", ")) };
			let run = if *count > 1 { format!(" ×{count}") } else { String::new() };
			format!("{}{run}{notes}", short(hash))
		})
		.collect::<Vec<_>>()
		.join(", ")
}

/// Collapse consecutive identical entries. Distinct runs stay distinct: a core mid-handover holds
/// a mixture, and that mixture is exactly what says how far along it is.
fn runs(entries: &[AuthorizerHash]) -> Vec<(AuthorizerHash, usize)> {
	let mut runs: Vec<(AuthorizerHash, usize)> = Vec::new();
	for entry in entries {
		match runs.last_mut() {
			Some((hash, count)) if hash == entry => *count += 1,
			_ => runs.push((*entry, 1)),
		}
	}
	runs
}

/// `0xabcd…1234`: enough of a hash to tell two apart at a glance, short enough for a table.
fn short(hash: &AuthorizerHash) -> String {
	let full = hex(&hash.0);
	format!("0x{}…{}", &full[..4], &full[full.len() - 4..])
}

/// Names for the authorizer hashes this tool can reproduce.
///
/// Only ever derived, never guessed: a name is attached because the hash was recomputed from a
/// config and a code hash the tool holds, so it is a preimage match. Anything else stays a bare
/// hash rather than an assumption the reader would then act on. That is also why sweeping every
/// candidate credential is safe rather than reckless — a wrong candidate derives hashes the chain
/// does not hold, and a right one proves itself.
fn names(credentials: &[Aura]) -> Vec<(AuthorizerHash, String)> {
	let mut names = vec![(cores::unassigned(), "unassigned, genesis".to_string())];
	for aura in credentials {
		let credential = format!("{}, {}", aura.scheme, aura.collators);
		names.push((aura.parked_hash(), format!("parked, {credential}")));
		names.extend((0..LABELLED_PARAS).map(|para| {
			(aura.hash(ParaId(para)), format!("para {para}, {credential}"))
		}));
	}
	names
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::aura::Scheme;
	use jam_types::CodeHash;

	fn hash(byte: u8) -> AuthorizerHash {
		AuthorizerHash([byte; 32])
	}

	fn aura(collators: &str, scheme: Scheme) -> Aura {
		Aura::from_dev_names(collators, scheme, CodeHash::zero(), 5, 1)
			.expect("dev names derive; qed")
	}

	/// A queue is eighty copies of one hash and a pool eight, so collapsing is what makes the
	/// output readable at all — but a core mid-handover is the interesting case, and there the
	/// boundary between the runs is the whole story.
	#[test]
	fn runs_collapse_but_a_handover_stays_visible_works() {
		assert_eq!(runs(&[hash(1); 3]), vec![(hash(1), 3)]);
		assert_eq!(
			runs(&[hash(1), hash(1), hash(2), hash(2), hash(2)]),
			vec![(hash(1), 2), (hash(2), 3)]
		);
		// Non-adjacent repeats are two runs, not one: order is what says which is arriving.
		assert_eq!(
			runs(&[hash(1), hash(2), hash(1)]),
			vec![(hash(1), 1), (hash(2), 1), (hash(1), 1)]
		);
		assert!(runs(&[]).is_empty());
	}

	/// A label is only ever attached to a hash the tool recomputed, and it has to survive the run
	/// collapsing — an unlabelled hash next to a labelled one is how a core assigned by someone
	/// else shows up.
	#[test]
	fn only_derivable_hashes_are_named_works() {
		let names = vec![(hash(1), "para 0, sr25519".to_string())];
		assert_eq!(describe(&[hash(1); 8], &names, None), "0x0101…0101 ×8 (para 0, sr25519)");
		assert_eq!(describe(&[hash(9)], &names, None), "0x0909…0909");
		assert_eq!(
			describe(&[hash(9), hash(1)], &names, None),
			"0x0909…0909, 0x0101…0101 (para 0, sr25519)"
		);
		assert_eq!(describe(&[], &names, None), "(empty)");
	}

	/// Sweeping many candidate credentials widens what *can* be named, never what *is*: a hash is
	/// named because some candidate reproduced it, so a hash no candidate reproduces stays bare
	/// however many candidates were tried. The label has to say which credential matched, or a
	/// reader could not act on it.
	#[test]
	fn a_wider_sweep_still_only_names_what_it_reproduced_works() {
		let credentials = vec![aura("alice,bob", Scheme::Sr25519), aura("alice", Scheme::Ed25519)];
		let names = names(&credentials);
		let (sr, ed) = (&credentials[0], &credentials[1]);

		assert_eq!(
			describe(&[sr.hash(ParaId(3))], &names, None),
			format!("{} (para 3, sr25519, alice,bob)", short(&sr.hash(ParaId(3))))
		);
		assert_eq!(
			describe(&[ed.parked_hash()], &names, None),
			format!("{} (parked, ed25519, alice)", short(&ed.parked_hash()))
		);
		// A credential nobody offered names nothing, and neither does an arbitrary hash.
		let unknown = aura("charlie", Scheme::Sr25519);
		assert_eq!(
			describe(&[unknown.hash(ParaId(0)), hash(9)], &names, None),
			format!("{}, 0x0909…0909", short(&unknown.hash(ParaId(0))))
		);
	}

	/// The watch loop reprints when the *state* moved, not when the block did. The rendering is
	/// dated with its block, so comparing renderings would call every block a change — which is
	/// the whole of what `--watch` is supposed to filter out.
	#[test]
	fn a_new_block_alone_is_not_a_change_works() {
		let steady: Cores = vec![(vec![hash(1); 8], vec![hash(1); 80])];
		// What two samples of an unmoved chain look like: same content, different blocks.
		let again: Cores = vec![(vec![hash(1); 8], vec![hash(1); 80])];
		let moved: Cores = vec![(vec![hash(2); 8], vec![hash(1); 80])];
		assert_eq!(steady, again);
		assert_ne!(steady, moved);
		assert_ne!(
			render(HeaderHash::from([1u8; 32]), &steady, &[]),
			render(HeaderHash::from([2u8; 32]), &steady, &[])
		);
	}

	/// A pool entry the core's queue no longer holds can never be refilled: it is a leftover from
	/// an earlier assignment, and on a core that reports every block it can sit there forever.
	/// Reading that as "the handover is stuck" is exactly the wrong conclusion, so say which
	/// entries are already gone from the queue.
	#[test]
	fn a_pool_entry_the_queue_cannot_refill_reads_as_stale_works() {
		let queue = vec![hash(2); 80];
		let pool = [vec![hash(1); 3], vec![hash(2); 5]].concat();
		assert_eq!(
			describe(&pool, &[], Some(&queue[..])),
			"0x0101…0101 ×3 (stale), 0x0202…0202 ×5"
		);
		// The queue itself is not judged against anything: every entry in it is by definition
		// still on its way to the pool.
		assert_eq!(describe(&queue, &[], None), "0x0202…0202 ×80");
	}
}
