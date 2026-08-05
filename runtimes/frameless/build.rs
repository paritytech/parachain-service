// Same as the FRAME runtime's `build_using_defaults()`, except that a FRAME-less
// runtime has no `impl_runtime_apis!` and therefore no `runtime_version` section for
// `substrate-wasm-builder` to find — so that one check is disabled. Everything else
// (import_memory + export_heap_base) is the substrate default, and
// `SUBSTRATE_RUNTIME_TARGET=riscv` (set in `.cargo/config.toml`) makes it emit a
// PolkaVM blob rather than WASM.
#[cfg(feature = "std")]
fn main() {
	substrate_wasm_builder::WasmBuilder::init_with_defaults()
		.disable_runtime_version_section_check()
		.build();
}

#[cfg(not(feature = "std"))]
fn main() {}
