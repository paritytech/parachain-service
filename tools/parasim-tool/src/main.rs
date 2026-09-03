//! Inspect and drive parasim on a running JAM testnet.
//!
//! A debugging companion to the parasim service: it speaks the same wire formats and reuses the
//! same verifier, so what it shows is what refine would see.

use std::path::{Path, PathBuf};

use clap::{Args as ClapArgs, Parser, Subcommand};
use cumulus_jam_interface::{CoreIndex, ServiceId};
use jam_types::CodeHash;
use parachain_service_interface::types::ParaId;

mod aura;
mod authorizers;
mod bootstrap;
mod chain;
mod control;
mod cores;
mod deploy;
mod format;
mod header;
mod inflight;
mod keys;
mod package;
mod rpc;
mod send;

#[derive(Parser)]
#[command(about, version)]
struct Cli {
	/// The JAM node's RPC endpoint. Must be ws:// or wss://.
	#[arg(long, env = "JAM_RPC", default_value = "ws://127.0.0.1:19800", global = true)]
	rpc: String,
	/// The parasim service to talk to.
	#[arg(long, default_value_t = 5, global = true)]
	service: ServiceId,
	#[command(flatten)]
	aura: AuraArgs,
	#[command(subcommand)]
	command: Command,
}

/// How the AURA authorizer a package runs under is put together. Every field is committed to by
/// the authorizer hash a core's queue holds, so all of them have to match what the collator uses
/// or the resulting hash is one nobody will ever install.
#[derive(ClapArgs)]
struct AuraArgs {
	/// The AURA authorizer blob, for its code hash. Only its hash is read; `deploy-authorizer`
	/// is what puts the bytes on chain.
	#[arg(long, env = "AUTHORIZER_BLOB", global = true, value_name = "PATH")]
	authorizer_blob: Option<PathBuf>,
	/// The collator set, as dev names in round-robin order.
	#[arg(long, default_value = "alice", global = true, value_name = "NAMES")]
	collators: String,
	/// The curve the collators sign on: the para runtime's `AuraId`. Must match
	/// --authorizer-blob, which nothing can check — a blob's scheme is not visible in its bytes,
	/// and a mismatch shows up only as a core no collator ever authorizes on.
	#[arg(long, value_enum, default_value_t = aura::Scheme::Sr25519, global = true)]
	scheme: aura::Scheme,
	/// Length of a para slot, in JAM timeslots.
	#[arg(long, default_value_t = 1, global = true)]
	slot_duration: u32,
}

/// Substrate's dev accounts, which is the whole universe `--collators` can name a collator from.
const DEV_COLLATORS: [&str; 6] = ["alice", "bob", "charlie", "dave", "eve", "ferdie"];

impl AuraArgs {
	fn resolve(&self, service: ServiceId) -> Result<aura::Aura, String> {
		self.resolve_as(service, self.authorizer_blob.as_deref(), &self.collators, self.scheme)
	}

	/// Every credential a hash on the chain could have been built from, for `display-authorizers`
	/// to name hashes with: both curves and every dev collator set, against the blob this tool was
	/// given and every authorizer blob its own build left behind.
	///
	/// Sweeping is safe because a label is only ever attached to a hash that was *reproduced*: a
	/// candidate with the wrong curve, the wrong set or the wrong blob derives hashes the chain
	/// does not hold and so names nothing. `--service` and `--slot-duration` are not swept — they
	/// describe the network the operator says they are looking at, not a guess about it.
	fn candidates(&self, service: ServiceId) -> Vec<aura::Aura> {
		let mut candidates = Vec::new();
		for code_hash in self.candidate_code_hashes() {
			for scheme in [aura::Scheme::Sr25519, aura::Scheme::Ed25519] {
				for set in dev_collator_sets(scheme, &self.collators) {
					let slot_duration = self.slot_duration;
					let credential =
						aura::Aura::from_dev_names(&set, scheme, code_hash, service, slot_duration);
					candidates.extend(credential.ok());
				}
			}
		}
		candidates
	}

	/// The code hashes worth trying: the blob the operator named, then every authorizer blob this
	/// tool's own build left in the target directory.
	///
	/// The built ones are worth trying because PVM builds are not byte-deterministic — a rebuild
	/// hashes differently from the copy that went on chain — so which of them, if any, is the
	/// deployed one is decided by whether its derived hash matches, not by picking one.
	fn candidate_code_hashes(&self) -> Vec<CodeHash> {
		let mut hashes = Vec::new();
		for path in self.authorizer_blob.iter().cloned().chain(built_authorizer_blobs()) {
			// An empty file is a PVM build that was skipped, not a blob; hashing it would offer a
			// candidate that can never match anything.
			let Ok(blob) = std::fs::read(&path) else { continue };
			if blob.is_empty() {
				continue;
			}
			let hash = CodeHash::from(jam_std_common::hash_raw(&blob));
			if !hashes.contains(&hash) {
				hashes.push(hash);
			}
		}
		hashes
	}

	/// Build a credential for some other para's authorizer — the carrier's, where each `--via-*`
	/// override falls back to what the target para uses.
	fn resolve_as(
		&self,
		service: ServiceId,
		blob: Option<&Path>,
		collators: &str,
		scheme: aura::Scheme,
	) -> Result<aura::Aura, String> {
		let path = blob.or(self.authorizer_blob.as_deref()).ok_or(
			"this command needs the AURA authorizer's code hash: pass --authorizer-blob (or set \
			 AUTHORIZER_BLOB) to the blob `deploy-authorizer` put on chain",
		)?;
		let blob = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
		let code_hash = jam_std_common::hash_raw(&blob).into();
		aura::Aura::from_dev_names(collators, scheme, code_hash, service, self.slot_duration)
	}
}

#[derive(Subcommand)]
enum Command {
	/// List recent blocks, newest first.
	DisplayChain {
		/// How many blocks to show.
		#[arg(short = 'N', long = "number", default_value_t = 5)]
		count: usize,
		/// Follow the finalized chain instead of the best chain.
		#[arg(long)]
		finalized: bool,
	},
	/// Read and decode a service-storage entry.
	DisplayKey {
		#[command(subcommand)]
		subject: Key,
	},
	/// Show what each core's authorizer pool and queue hold.
	///
	/// The queue is what `assign` writes; the pool is what a package can actually be reported
	/// under, and it refills from the queue one entry per block. Hashes the tool can reproduce are
	/// named — it tries both curves and every dev collator set against --authorizer-blob and any
	/// authorizer blob its own build left behind — and the rest are shown as they are.
	DisplayAuthorizers {
		/// Read at this block instead of the current best.
		#[arg(long, value_name = "HASH", conflicts_with = "watch")]
		block: Option<String>,
		/// Sample every slot and re-print whenever a pool or queue moves.
		#[arg(long)]
		watch: bool,
	},
	/// Show work packages JAM has reported but not yet accumulated.
	///
	/// Two things this cannot do: state for older blocks may have been pruned, and each read is a
	/// snapshot, so a package reported and accumulated between two samples never shows up. Use
	/// `--watch` alongside `send` in another terminal.
	DisplayInflight {
		/// Read at this block instead of the current best.
		#[arg(long, value_name = "HASH", conflicts_with = "watch")]
		block: Option<String>,
		/// Only show packages for this para.
		#[arg(long)]
		para: Option<u32>,
		/// Sample every slot and print packages as they arrive and leave.
		#[arg(long)]
		watch: bool,
		/// Print the undecoded refine output instead of interpreting it.
		#[arg(long)]
		raw: bool,
	},
	/// Host the AURA authorizer blob in the bootstrap service: solicit it, then provide it.
	DeployAuthorizer {
		/// The blob to deploy. Defaults to --authorizer-blob.
		#[arg(value_name = "PATH")]
		blob: Option<PathBuf>,
	},
	/// Hand a core's assigner privilege to parasim. Run it after the core's first `assign-core`:
	/// once parasim owns a core, only a control package on an AURA core can assign it.
	///
	/// The grant itself is a bootstrap instruction, so it always rides a core still holding the
	/// genesis authorizer — there is no AURA lane for it. Assignment to parasim is one-way, so
	/// there is no way to make such a core again: grant every core parasim is to own while one is
	/// still unassigned.
	GrantAssigner {
		/// Which core.
		core: CoreIndex,
	},
	/// Point a core's authorizer queue at a para's AURA authorizer.
	AssignCore {
		/// Which para.
		para: u32,
		/// Which core.
		core: CoreIndex,
		#[command(flatten)]
		via: Via,
	},
	/// Park a core: no para on it, but the same AURA authorizer, so it still takes commands.
	///
	/// The parked authorizer commits to `--collators`/`--scheme`, so pass the ones the core was
	/// assigned with. Its pool drains over the next few blocks, after which the core refuses
	/// parachain work and `assign-core` can put a para back on it, riding the core itself.
	FreeCore {
		/// Which core.
		core: CoreIndex,
		#[command(flatten)]
		via: Via,
	},
	/// Submit mock work packages and follow them until the head moves.
	Send {
		/// The para to build for.
		#[arg(long, default_value_t = 0)]
		para: u32,
		/// The core to submit to.
		#[arg(long, default_value_t = 0)]
		core: u16,
		/// How many linked packages to send. Each one after the first builds on a block that is
		/// still in flight, so only accumulate's reorder buffer can put it back in order.
		#[arg(long, default_value_t = 1)]
		chain: usize,
		/// Plant a defect the para must not accept. Bare `--tamper` corrupts the state proof.
		#[arg(long, value_name = "KIND", value_enum, num_args = 0..=1, default_missing_value = "proof")]
		tamper: Option<send::Tamper>,
		/// Which package of the chain to tamper with, counted from zero.
		#[arg(long, value_name = "INDEX", default_value_t = 0, requires = "tamper")]
		tamper_at: usize,
	},
}

/// Which core a control package rides, once parasim is the assigner and the command has to come
/// from it. Not necessarily the core being assigned: any core under an authorizer this tool can
/// sign for will carry the command.
///
/// The carrier is a whole authorizer of its own, so the credential is too: the token must hash to
/// exactly what the carrier core's queue holds, which is the *carrier* para's collator set on the
/// *carrier* para's curve. That is the same as the target's only when one collator set runs
/// everything.
#[derive(ClapArgs, Clone)]
struct Via {
	/// The core to submit the control package to. Defaults to the core being assigned.
	#[arg(long, value_name = "CORE")]
	via_core: Option<CoreIndex>,
	/// The para that core is currently running, whose collator has to sign the package.
	#[arg(long, value_name = "PARA", default_value_t = 0)]
	via_para: u32,
	/// The carrier para's collator set. Defaults to --collators.
	#[arg(long, value_name = "NAMES")]
	via_collators: Option<String>,
	/// The carrier para's curve. Defaults to --scheme.
	#[arg(long, value_enum)]
	via_scheme: Option<aura::Scheme>,
	/// The carrier para's authorizer blob, when it is not on the same curve. Defaults to
	/// --authorizer-blob.
	#[arg(long, value_name = "PATH")]
	via_authorizer_blob: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Key {
	/// The para's head, as stored in the service's `parachains` map.
	Parahead(KeyArgs),
	/// The para's reorder buffer: heads accumulate has parked until their parent arrives.
	Buffer(KeyArgs),
}

/// The same three options for every subject: which para, when, and how much decoding.
#[derive(ClapArgs)]
struct KeyArgs {
	/// Which para.
	para: u32,
	/// Read at this block instead of the current best.
	#[arg(long, value_name = "HASH", conflicts_with = "watch")]
	block: Option<String>,
	/// Sample every slot and re-print whenever the entry changes.
	#[arg(long)]
	watch: bool,
	/// Print the stored bytes without decoding them.
	#[arg(long)]
	raw: bool,
}

/// The collator sets worth trying when naming a hash: the one the operator named, every dev
/// singleton, and every prefix of the dev accounts both in the order they are written and in the
/// order a runtime hands them back. The last two differ as soon as a set has more than one member,
/// and they are different authorizers — see [`aura::in_authority_order`].
fn dev_collator_sets(scheme: aura::Scheme, named: &str) -> Vec<String> {
	let mut sets = vec![named.to_string()];
	for size in 1..=DEV_COLLATORS.len() {
		sets.push(DEV_COLLATORS[..size].join(","));
		if let Ok(set) = aura::in_authority_order(&DEV_COLLATORS[..size], scheme) {
			sets.push(set);
		}
	}
	sets.extend(DEV_COLLATORS.iter().map(|name| name.to_string()));
	sets.sort();
	sets.dedup();
	sets
}

/// Authorizer blobs this tool's own build left in the target directory, found relative to the
/// running executable so that it does not matter where the tool is invoked from.
///
/// A build script's output lives under `build/<package>/<fingerprint>/out`, and every fingerprint
/// is worth reading: they are the blobs of every build this target directory still remembers, and
/// which of them went on chain is decided by whether its hash matches, not by picking the newest.
fn built_authorizer_blobs() -> Vec<PathBuf> {
	let Ok(build) = std::env::current_exe().map(|exe| exe.with_file_name("build")) else {
		return Vec::new();
	};
	let Ok(packages) = std::fs::read_dir(build) else { return Vec::new() };
	packages
		.flatten()
		.filter(|package| package.file_name().to_string_lossy().starts_with("parachain-authorizer"))
		.filter_map(|package| std::fs::read_dir(package.path()).ok())
		.flat_map(|fingerprints| fingerprints.flatten())
		.filter_map(|fingerprint| std::fs::read_dir(fingerprint.path().join("out")).ok())
		.flat_map(|out| out.flatten())
		.map(|blob| blob.path())
		.filter(|blob| blob.extension().is_some_and(|extension| extension == "jam"))
		.collect()
}

#[tokio::main]
async fn main() {
	// This tool's own progress lines go through `tracing`, and their timestamps are the whole
	// point: correlating what the tool did with what the node logged is otherwise done by hand.
	// So INFO for us and `warn` for the libraries, whose connection chatter would otherwise bury
	// it. Command *output* stays on bare stdout, undecorated, so piping it still works.
	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| "warn,parasim_tool=info".into()),
		)
		.init();

	let cli = Cli::parse();
	if let Err(error) = run(cli).await {
		eprintln!("error: {error}");
		std::process::exit(1);
	}
}

async fn run(cli: Cli) -> Result<(), String> {
	let jam = rpc::connect(&cli.rpc).await?;
	match cli.command {
		Command::DisplayChain { count, finalized } => chain::display(&jam, count, finalized).await,
		Command::DisplayInflight { block, para, watch, raw } => {
			let args = inflight::Args { service: cli.service, para, block, watch, raw };
			inflight::run(&jam, &args).await
		},
		Command::DisplayKey { subject } => {
			let (subject, key) = match subject {
				Key::Parahead(key) => (keys::Subject::Parahead, key),
				Key::Buffer(key) => (keys::Subject::Buffer, key),
			};
			let args = keys::Args {
				service: cli.service,
				para: ParaId(key.para),
				subject,
				block: key.block,
				watch: key.watch,
				raw: key.raw,
			};
			keys::run(&jam, &args).await
		},
		Command::DisplayAuthorizers { block, watch } => {
			// Naming a hash needs a code hash, and a missing one is not an error here: the genesis
			// authorizer is derivable without any blob at all, and the rest stay bare hashes.
			let args =
				authorizers::Args { block, watch, credentials: cli.aura.candidates(cli.service) };
			authorizers::run(&jam, &args).await
		},
		Command::DeployAuthorizer { blob } => {
			let blob = blob
				.or_else(|| cli.aura.authorizer_blob.clone())
				.ok_or("pass the authorizer blob, or set --authorizer-blob/AUTHORIZER_BLOB")?;
			deploy::run(&jam, &blob).await
		},
		Command::GrantAssigner { core } =>
			control::grant(&jam, &control_args(&cli, None)?, core).await,
		Command::AssignCore { para, core, ref via } =>
			control::assign(&jam, &control_args(&cli, Some(via))?, ParaId(para), core).await,
		Command::FreeCore { core, ref via } =>
			control::free(&jam, &control_args(&cli, Some(via))?, core).await,
		Command::Send { para, core, chain, tamper, tamper_at } => {
			let args = send::Args {
				service: cli.service,
				para: ParaId(para),
				core,
				chain,
				tamper,
				tamper_at,
				aura: cli.aura.resolve(cli.service)?,
			};
			send::run(&jam, &args).await
		},
	}
}

/// `grant-assigner` never needs a carrier core, so it passes no `--via`. For the other two a
/// wrong carrier credential costs nothing: the carrier's queue must hold exactly the authorizer
/// the token is built for, and that is checked before anything is submitted.
fn control_args(cli: &Cli, via: Option<&Via>) -> Result<control::Args, String> {
	let carrier = match via {
		Some(via) => cli.aura.resolve_as(
			cli.service,
			via.via_authorizer_blob.as_deref(),
			via.via_collators.as_deref().unwrap_or(&cli.aura.collators),
			via.via_scheme.unwrap_or(cli.aura.scheme),
		)?,
		None => cli.aura.resolve(cli.service)?,
	};
	Ok(control::Args {
		service: cli.service,
		aura: cli.aura.resolve(cli.service)?,
		carrier,
		via_core: via.and_then(|via| via.via_core),
		via_para: ParaId(via.map_or(0, |via| via.via_para)),
	})
}
