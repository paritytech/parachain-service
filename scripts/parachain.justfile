import 'common.justfile'

# Disassemble the parachain runtime blobs and list their PVM exports.
entrypoints:
	polkatool disassemble {{ ASSET_HUB_BLOB }} | grep "export"
	polkatool disassemble {{ CORETIME_BLOB }} | grep "export"

# Print a runtime blob's `Core_version` (exercises the WasmExecutor path).
version blob=ASSET_HUB_BLOB:
	cargo run --package {{ EXECUTOR_PACKAGE }} --features executor -- runtime --blob {{ blob }} core-version

# Run the Asset Hub runtime-owned refine test.
test *ARGS:
	SKIP_WASM_BUILD=1 cargo test --package asset-hub {{ ARGS }}
