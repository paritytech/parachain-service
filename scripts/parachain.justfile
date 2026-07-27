import 'common.justfile'

# Disassemble the parachain runtime blobs and list their PVM exports.
entrypoints:
	polkatool disassemble {{ ASSET_HUB_BLOB }} | grep "export"
	polkatool disassemble {{ CORETIME_BLOB }} | grep "export"

# Print a runtime blob's `Core_version` (exercises the WasmExecutor path).
version blob=ASSET_HUB_BLOB:
	cargo run --manifest-path {{ EXECUTOR }} -- runtime --blob {{ blob }} core-version
