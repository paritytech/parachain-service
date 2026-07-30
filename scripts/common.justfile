EXECUTOR_PACKAGE := "executor-cli"

ASSET_HUB_BLOB := justfile_directory() / "target/release/rbuild/asset-hub/asset-hub-blob.polkavm"
CORETIME_BLOB := justfile_directory() / "target/release/rbuild/coretime/coretime-blob.polkavm"

SERVICE_BLOB := justfile_directory() / "target/parachain-service.jam"
AUTHORIZER_BLOB := justfile_directory() / "target/parachain-authorizer.jam"

MAX_BLOB_SIZE := "4194304"

# Build-critical vendored dependency that `tools/executor` builds against by path.
# Unlike `polkadot-sdk-companion` it is NOT a git submodule, so pin it here and
# fetch it via `just vendor`. Bump both together when upgrading polkajam.
POLKAJAM_URL := "https://github.com/paritytech/polkajam.git"
POLKAJAM_REV := "22d38b14a4d1c84d3765ad59f78e26f01747269c"
