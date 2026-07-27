# PVM entrypoints
entrypoints:
	polkatool disassemble ../target/release/rbuild/asset-hub/asset-hub-blob.polkavm | grep "export"
	polkatool disassemble ../target/release/rbuild/coretime/coretime-blob.polkavm | grep "export"

version blob=(justfile_directory() / "target/release/rbuild/asset-hub/asset-hub-blob.polkavm"):
	cargo run --manifest-path {{ justfile_directory() / "tools/runtime-executor/Cargo.toml" }} \
		-- --blob {{ blob }} core-version
