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
	/// The credential whose hashes can be named, when the tool was given an authorizer blob.
	/// Without one only the genesis authorizer is derivable.
	pub aura: Option<Aura>,
}

pub async fn run(jam: &JamRpcInterface, args: &Args) -> Result<(), String> {
	let names = names(args.aura.as_ref());
	if args.watch {
		return watch(jam, &names).await;
	}

	let at = match &args.block {
		Some(hash) => crate::format::parse_header_hash(hash)?,
		None => jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash,
	};
	print!("{}", sample(jam, at, &names).await?);
	Ok(())
}

/// Poll once per slot and re-print when anything moved.
///
/// Same convention as `display-inflight --watch`: one verb, `--watch` streams changes. A core
/// takes several blocks to hand over — the queue moves at once, the pool one entry at a time —
/// and watching is the only way to see that happen rather than infer it.
async fn watch(jam: &JamRpcInterface, names: &[(AuthorizerHash, String)]) -> Result<(), String> {
	let VersionedParameters::V1(parameters) =
		jam.parameters().await.map_err(|e| format!("parameters: {e}"))?;
	let mut previous = String::new();
	loop {
		let best = jam.best_block().await.map_err(|e| format!("best block: {e}"))?;
		let current = sample(jam, best.header_hash, names).await?;
		if current != previous {
			tracing::info!("authorizers changed at block 0x{}", hex(&*best.header_hash));
			print!("{current}");
			previous = current;
		}
		tokio::time::sleep(Duration::from_secs(parameters.slot_period_sec.into())).await;
	}
}

/// Every core's pool and queue at `at`, rendered. Returned as one string so the watch loop can
/// compare two samples without caring what is in them.
async fn sample(
	jam: &JamRpcInterface,
	at: HeaderHash,
	names: &[(AuthorizerHash, String)],
) -> Result<String, String> {
	use std::fmt::Write as _;

	let pools = jam.auth_pools(at).await.map_err(|e| format!("reading the pools: {e}"))?;
	let queues = jam.auth_queues(at).await.map_err(|e| format!("reading the queues: {e}"))?;

	let mut out = String::new();
	let _ = writeln!(out, "block 0x{}", hex(&*at));
	for (core, (pool, queue)) in pools.iter().zip(queues.iter()).enumerate() {
		let _ = writeln!(out, "core {core}");
		let _ = writeln!(out, "  pool  {}", describe(pool, names));
		let _ = writeln!(out, "  queue {}", describe(&queue[..], names));
	}
	Ok(out)
}

/// One core's pool or queue as a line: runs of the same hash collapsed, each named where the tool
/// can reproduce it.
fn describe(entries: &[AuthorizerHash], names: &[(AuthorizerHash, String)]) -> String {
	if entries.is_empty() {
		return "(empty)".to_string();
	}
	runs(entries)
		.iter()
		.map(|(hash, count)| {
			let name = names
				.iter()
				.find(|(known, _)| known == hash)
				.map_or(String::new(), |(_, name)| format!(" ({name})"));
			format!("{}{}{name}", short(hash), if *count > 1 { format!(" ×{count}") } else { String::new() })
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
/// hash rather than an assumption the reader would then act on.
fn names(aura: Option<&Aura>) -> Vec<(AuthorizerHash, String)> {
	let mut names = vec![(cores::unassigned(), "unassigned, genesis".to_string())];
	let Some(aura) = aura else { return names };

	names.push((aura.parked_hash(), format!("parked, {}", aura.scheme)));
	names.extend(
		(0..LABELLED_PARAS)
			.map(|para| (aura.hash(ParaId(para)), format!("para {para}, {}", aura.scheme))),
	);
	names
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hash(byte: u8) -> AuthorizerHash {
		AuthorizerHash([byte; 32])
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
		assert_eq!(describe(&[hash(1); 8], &names), "0x0101…0101 ×8 (para 0, sr25519)");
		assert_eq!(describe(&[hash(9)], &names), "0x0909…0909");
		assert_eq!(
			describe(&[hash(9), hash(1)], &names),
			"0x0909…0909, 0x0101…0101 (para 0, sr25519)"
		);
		assert_eq!(describe(&[], &names), "(empty)");
	}
}
