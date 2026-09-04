//! Compiles the `parachain-authorizer-sr25519` guest crate into a PVM blob.
//!
//! `jam_pvm_builder::build_authorizer` runs an inner `cargo rustc` for the PVM
//! target, writes the `.jam` blob into `OUT_DIR`, and exports its path via the
//! `PVM_BINARY_parachain-authorizer-sr25519` env var (consumed by `pvm_binary!` in
//! `src/lib.rs`). It also emits `cargo:rerun-if-changed` for the guest crate, so
//! `cargo build`/`cargo test` rebuild the blob whenever the guest source changes.
//!
//! Set `SKIP_PVM_BUILDS=1` to emit an empty dummy blob instead (fast `cargo check`).

fn main() {
	pvm_builder::build_authorizer(parachain_authorizer_sr25519::MANIFEST_DIR.as_ref());
}
