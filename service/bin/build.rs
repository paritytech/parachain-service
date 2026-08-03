//! Compiles the `parachain-service` guest crate into a PVM blob.
//!
//! `jam_pvm_builder::build_service` runs an inner `cargo rustc` for the PVM target,
//! writes the `.jam` blob into `OUT_DIR`, and exports its path via the
//! `PVM_BINARY_parachain-service` env var (consumed by `pvm_binary!` in
//! `src/lib.rs`). It also emits `cargo:rerun-if-changed` for the guest crate, so
//! `cargo build`/`cargo test` rebuild the blob whenever the guest source changes.
//!
//! Set `SKIP_PVM_BUILDS=1` to emit an empty dummy blob instead (fast `cargo check`).

fn main() {
	jam_pvm_builder::build_service(parachain_service::MANIFEST_DIR.as_ref());
}
