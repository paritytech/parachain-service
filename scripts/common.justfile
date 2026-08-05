FRAMELESS_BLOB := justfile_directory() / "target/release/rbuild/frameless/frameless-blob.polkavm"

SERVICE_BLOB := justfile_directory() / "target/parachain-service.jam"
AUTHORIZER_BLOB := justfile_directory() / "target/parachain-authorizer.jam"

MAX_BLOB_SIZE := "4194304"
