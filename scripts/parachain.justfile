import 'common.justfile'

# Disassemble the parachain runtime blobs and list their PVM exports.
entrypoints:
	polkatool disassemble {{ ASSET_HUB_BLOB }} | grep "export"
	polkatool disassemble {{ CORETIME_BLOB }} | grep "export"

# Run the Asset Hub runtime-owned refine test.
test *ARGS:
	SKIP_WASM_BUILD=1 cargo test --package asset-hub {{ ARGS }}
