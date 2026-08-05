import 'common.justfile'

# Disassemble the parachain runtime blob and list its PVM exports.
entrypoints:
	polkatool disassemble {{ FRAMELESS_BLOB }} | grep "export"

# Run the frameless runtime-owned refine test.
test *ARGS:
	SKIP_WASM_BUILD=1 cargo test --package frameless {{ ARGS }}
