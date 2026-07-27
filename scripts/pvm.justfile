# PVM entrypoints
entrypoints:
	polkatool disassemble ../target/release/rbuild/asset-hub/asset-hub-blob.polkavm | grep "export"
	polkatool disassemble ../target/release/rbuild/coretime/coretime-blob.polkavm | grep "export"
