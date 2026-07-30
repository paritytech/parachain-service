//! Runtime backend: run a Substrate *runtime* blob (`*.polkavm`) via `WasmExecutor`.
//!
//! `WasmExecutor` transparently routes PolkaVM blobs to the `sc-executor-polkavm`
//! backend and wires up the full `sp_io` host-function set the runtime imports,
//! resolving imports *by name*. It expects the Substrate runtime ABI (call export
//! by name, args `(ptr, len)`, packed-`u64` return). This is the right tool for the
//! runtime blob and the wrong one for a JAM service (see `service.rs`).

use std::borrow::Cow;

use anyhow::{anyhow, Result};
use codec::Decode;
use sc_executor::WasmExecutor;
use sp_core::traits::{CallContext, CodeExecutor, FetchRuntimeCode, RuntimeCode};
use sp_runtime::traits::BlakeTwo256;
use sp_state_machine::TestExternalities;
use sp_version::RuntimeVersion;

/// Call `Core_version` and return the decoded [`RuntimeVersion`].
pub fn core_version(code: &[u8]) -> Result<RuntimeVersion> {
    let encoded = call_export(code, "Core_version", &[])?;
    RuntimeVersion::decode(&mut &encoded[..])
        .map_err(|e| anyhow!("failed to decode RuntimeVersion ({e})"))
}

/// Call an arbitrary exported method and return its raw SCALE-encoded output.
pub fn call(code: &[u8], method: &str, input: &[u8]) -> Result<Vec<u8>> {
    call_export(code, method, input)
}

/// Trivial [`FetchRuntimeCode`] over an in-memory blob.
struct CodeFetcher<'a>(&'a [u8]);
impl FetchRuntimeCode for CodeFetcher<'_> {
    fn fetch_runtime_code(&self) -> Option<Cow<'_, [u8]>> {
        Some(self.0.into())
    }
}

/// Instantiate `code` with the standard `sp_io` host functions and invoke `method`
/// with `data`, returning the raw SCALE-encoded output.
fn call_export(code: &[u8], method: &str, data: &[u8]) -> Result<Vec<u8>> {
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
    let (result, _used_native) = executor.call(
        &mut ext.ext(),
        &runtime_code,
        method,
        data,
        CallContext::Offchain,
    );
    result.map_err(|e| anyhow!("`{method}` execution failed: {e}"))
}
