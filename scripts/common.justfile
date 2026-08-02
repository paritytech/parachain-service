ASSET_HUB_BLOB := justfile_directory() / "target/release/rbuild/asset-hub/asset-hub-blob.polkavm"
CORETIME_BLOB := justfile_directory() / "target/release/rbuild/coretime/coretime-blob.polkavm"

SERVICE_BLOB := justfile_directory() / "target/parachain-service.jam"
AUTHORIZER_BLOB := justfile_directory() / "target/parachain-authorizer.jam"

MAX_BLOB_SIZE := "4194304"
