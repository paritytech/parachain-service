EXECUTOR := justfile_directory() / "tools/executor/Cargo.toml"

ASSET_HUB_BLOB := justfile_directory() / "target/release/rbuild/asset-hub/asset-hub-blob.polkavm"
CORETIME_BLOB := justfile_directory() / "target/release/rbuild/coretime/coretime-blob.polkavm"

SERVICE_BLOB := justfile_directory() / "target/parachain-service.jam"

MAX_BLOB_SIZE := "4194304"
