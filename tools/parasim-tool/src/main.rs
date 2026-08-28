//! Inspect and drive parasim on a running JAM testnet.
//!
//! A debugging companion to the parasim service: it speaks the same wire formats and reuses the
//! same verifier, so what it shows is what refine would see.

use clap::{Parser, Subcommand};
use jam_interface::ServiceId;
use parachain_service_interface::types::ParaId;

mod chain;
mod format;
mod keys;
mod rpc;
mod send;

#[derive(Parser)]
#[command(about, version)]
struct Cli {
	/// The JAM node's RPC endpoint. Must be ws:// or wss://.
	#[arg(long, env = "JAM_RPC", default_value = "ws://127.0.0.1:19800", global = true)]
	rpc: String,
	/// The parasim service to talk to.
	#[arg(long, default_value_t = 9, global = true)]
	service: ServiceId,
	#[command(subcommand)]
	command: Command,
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
	/// Submit a mock work package and follow it until the head moves.
	Send {
		/// The para to build for.
		#[arg(long, default_value_t = 0)]
		para: u32,
		/// The core to submit to.
		#[arg(long, default_value_t = 0)]
		core: u16,
		/// Corrupt the state proof, so refine must reject the package.
		#[arg(long)]
		tamper: bool,
	},
}

#[derive(Subcommand)]
enum Key {
	/// The para's head, as stored in the service's `parachains` map.
	Parahead {
		/// Which para.
		para: u32,
		/// Read at this block instead of the current best.
		#[arg(long, value_name = "HASH")]
		block: Option<String>,
		/// Print the stored bytes without decoding them.
		#[arg(long)]
		raw: bool,
	},
}

#[tokio::main]
async fn main() {
	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| "warn".into()),
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
		Command::DisplayChain { count, finalized } =>
			chain::display(&jam, count, finalized).await,
		Command::DisplayKey { subject: Key::Parahead { para, block, raw } } =>
			keys::display_parahead(&jam, cli.service, ParaId(para), block, raw).await,
		Command::Send { para, core, tamper } => {
			let args = send::Args { service: cli.service, para: ParaId(para), core, tamper };
			send::run(&jam, &args).await
		},
	}
}
