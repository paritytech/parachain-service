FRAMELESS_BLOB := justfile_directory() / "target/release/rbuild/frameless/frameless-blob.polkavm"

SERVICE_BLOB := justfile_directory() / "target/parachain-service.jam"
AUTHORIZER_ED25519_BLOB := justfile_directory() / "target/parachain-authorizer-ed25519.jam"
AUTHORIZER_SR25519_BLOB := justfile_directory() / "target/parachain-authorizer-sr25519.jam"

MAX_BLOB_SIZE := "4194304"
