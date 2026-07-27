//! CLI for executing parachain runtime PVM blobs natively.
//!
//! Runs a runtime blob (`*.polkavm`) through Substrate's [`WasmExecutor`], which
//! transparently routes PolkaVM blobs to the `sc-executor-polkavm` backend and
//! wires up the full `sp_io` host-function set the runtime imports.
//!
//! Use `core-version` for a quick check (proves the blob parses, the host
//! functions link, and an exported entrypoint executes), or `call` to invoke an
//! arbitrary export with a SCALE payload — e.g. `jam_validate_block` once it is
//! implemented with the `fn(ptr, len) -> u64` convention.

use std::{borrow::Cow, path::PathBuf};

use clap::{Parser, Subcommand};
use codec::Decode;
use sc_executor::WasmExecutor;
use sp_core::traits::{CallContext, CodeExecutor, FetchRuntimeCode, RuntimeCode};
use sp_runtime::traits::BlakeTwo256;
use sp_state_machine::TestExternalities;
use sp_version::RuntimeVersion;

#[derive(Parser)]
#[command(
    name = "runtime-executor",
    about = "Execute parachain runtime PVM blobs via Substrate's WasmExecutor (PolkaVM backend)"
)]
struct Cli {
    /// Path to the runtime PVM blob (`*.polkavm`).
    #[arg(long, short)]
    blob: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Call `Core_version` and print the decoded `RuntimeVersion`.
    CoreVersion,
    /// Call an arbitrary exported method with an optional hex-encoded payload.
    Call {
        /// Export to call, e.g. `Metadata_metadata` or `jam_validate_block`.
        method: String,
        /// Hex-encoded input payload (leading `0x` optional). Defaults to empty.
        #[arg(long)]
        input: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // The executor rejects PolkaVM blobs unless this is set (see
    // `sc_executor_common::is_polkavm_enabled`). Set before any executor use.
    std::env::set_var("SUBSTRATE_ENABLE_POLKAVM", "1");

    let code = std::fs::read(&cli.blob).map_err(|e| {
        anyhow::anyhow!("failed to read runtime blob at {} ({e})", cli.blob.display())
    })?;
    println!("Loaded runtime blob: {} ({} bytes)", cli.blob.display(), code.len());

    match cli.command {
        Command::CoreVersion => {
            let encoded = call_export(&code, "Core_version", &[])?;
            let version = RuntimeVersion::decode(&mut &encoded[..])
                .map_err(|e| anyhow::anyhow!("failed to decode RuntimeVersion ({e})"))?;

            println!("\n✅ Core_version executed successfully:");
            println!("  spec_name:           {}", version.spec_name);
            println!("  impl_name:           {}", version.impl_name);
            println!("  authoring_version:   {}", version.authoring_version);
            println!("  spec_version:        {}", version.spec_version);
            println!("  impl_version:        {}", version.impl_version);
            println!("  transaction_version: {}", version.transaction_version);
            println!("  runtime APIs:        {}", version.apis.len());
        },
        Command::Call { method, input } => {
            let input = match input {
                Some(hex_str) => hex::decode(hex_str.trim_start_matches("0x"))
                    .map_err(|e| anyhow::anyhow!("invalid --input hex ({e})"))?,
                None => Vec::new(),
            };
            let output = call_export(&code, &method, &input)?;
            println!("\n✅ {method} executed successfully, returned {} bytes:", output.len());
            println!("0x{}", hex::encode(&output));
        },
    }

    Ok(())
}

/// Trivial [`FetchRuntimeCode`] over an in-memory blob.
struct CodeFetcher<'a>(&'a [u8]);
impl FetchRuntimeCode for CodeFetcher<'_> {
    fn fetch_runtime_code(&self) -> Option<Cow<'_, [u8]>> {
        Some(self.0.into())
    }
}

/// Instantiate `code` with the standard `sp_io` host functions and invoke
/// `method` with `data`, returning the raw SCALE-encoded output.
fn call_export(code: &[u8], method: &str, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let executor = WasmExecutor::<sp_io::SubstrateHostFunctions>::builder()
        // The blob may be built against a different `sp-io` than the executor
        // provides; tolerate any host import we don't define (only faults if
        // actually called).
        .with_allow_missing_host_functions(true)
        .build();

    let code_fetcher = CodeFetcher(code);
    let runtime_code = RuntimeCode {
        code_fetcher: &code_fetcher,
        heap_pages: None,
        hash: sp_io::hashing::blake2_256(code).to_vec(),
    };

    let mut ext = TestExternalities::<BlakeTwo256>::default();
    let (result, _used_native) =
        executor.call(&mut ext.ext(), &runtime_code, method, data, CallContext::Offchain);
    result.map_err(|e| anyhow::anyhow!("`{method}` execution failed: {e}"))
}
