//! CLI for executing parachain PVM blobs natively, for debugging.

mod input;

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use executor::{
    host::DebugHost,
    runtime,
    service::{self, Entry},
};

#[derive(Parser)]
#[command(
    name = "executor",
    about = "Execute parachain PVM blobs natively (Substrate runtime + JAM service), for debugging"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a Substrate runtime blob (`*.polkavm`) via WasmExecutor.
    Runtime {
        /// Path to the runtime PVM blob.
        #[arg(long, short)]
        blob: PathBuf,
        #[command(subcommand)]
        action: RuntimeAction,
    },
    /// Run a JAM service blob (`*.jam`) on a bare PolkaVM engine.
    Service {
        /// Path to the service `.jam` blob.
        #[arg(long, short)]
        blob: PathBuf,
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum RuntimeAction {
    /// Call `Core_version` and print the decoded `RuntimeVersion`.
    CoreVersion,
    /// Call an arbitrary exported method with an optional hex payload.
    Call {
        /// Export to call, e.g. `Metadata_metadata` or `jam_validate_block`.
        method: String,
        /// Hex-encoded input payload (leading `0x` optional). Defaults to empty.
        #[arg(long)]
        input: Option<String>,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Invoke the `refine` entry point.
    Refine(ServiceArgs),
    /// Invoke the `accumulate` entry point.
    Accumulate(ServiceArgs),
}

#[derive(Args)]
struct ServiceArgs {
    /// Params input file: JSON, or raw SCALE bytes. Omit for zero defaults.
    #[arg(long, short)]
    input: Option<PathBuf>,
    /// Force the input format instead of inferring it from the file extension.
    #[arg(long, value_enum)]
    format: Option<input::Format>,
    /// Gas budget for the invocation.
    #[arg(long, default_value_t = 5_000_000_000)]
    gas: i64,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Runtime { blob, action } => {
            // sc-executor rejects PolkaVM blobs unless this is set.
            std::env::set_var("SUBSTRATE_ENABLE_POLKAVM", "1");
            let code = read_blob(&blob)?;
            match action {
                RuntimeAction::CoreVersion => {
                    let version = runtime::core_version(&code)?;
                    println!("\nCore_version executed successfully:");
                    println!("  spec_name:           {}", version.spec_name);
                    println!("  impl_name:           {}", version.impl_name);
                    println!("  authoring_version:   {}", version.authoring_version);
                    println!("  spec_version:        {}", version.spec_version);
                    println!("  impl_version:        {}", version.impl_version);
                    println!("  transaction_version: {}", version.transaction_version);
                    println!("  runtime APIs:        {}", version.apis.len());
                }
                RuntimeAction::Call { method, input } => {
                    let input = input
                        .map(|value| {
                            hex::decode(value.trim_start_matches("0x"))
                                .map_err(|error| anyhow!("invalid --input hex ({error})"))
                        })
                        .transpose()?
                        .unwrap_or_default();
                    let output = runtime::call(&code, &method, &input)?;
                    println!(
                        "\n{method} executed successfully, returned {} bytes:",
                        output.len()
                    );
                    println!("0x{}", hex::encode(output));
                }
            }
        }
        Command::Service { blob, action } => {
            let code = read_blob(&blob)?;
            let (entry, args) = match action {
                ServiceAction::Refine(args) => (Entry::Refine, args),
                ServiceAction::Accumulate(args) => (Entry::Accumulate, args),
            };

            let params = input::load_params(entry, args.input.as_deref(), args.format)?;
            println!(
                "Invoking `{}` with {} bytes of params (gas budget {})",
                entry.as_str(),
                params.len(),
                args.gas
            );

            let mut host = DebugHost;
            let outcome = service::run(&code, entry, &params, args.gas, &mut host)?;

            println!(
                "\n`{}` finished (gas used: {})",
                entry.as_str(),
                outcome.gas_used
            );
            println!(
                "  output ({} bytes): 0x{}",
                outcome.output.len(),
                hex::encode(&outcome.output)
            );
        }
    }
    Ok(())
}

fn read_blob(path: &Path) -> Result<Vec<u8>> {
    let code = std::fs::read(path).with_context(|| format!("reading blob {}", path.display()))?;
    println!("Loaded blob: {} ({} bytes)", path.display(), code.len());
    Ok(code)
}
