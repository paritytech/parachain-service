//! Compiles the `parasim-service` guest crate into a PVM blob.
//!
//! Same wiring as `service/bin/build.rs`: `pvm_builder::build_service` runs an
//! inner `cargo rustc` for the PVM target, writes the `.jam` blob into `OUT_DIR`,
//! and exports its path via `PVM_BINARY_parasim-service` (consumed by
//! `pvm_binary!` in `src/lib.rs`). `cargo build`/`cargo test` rebuild the blob
//! whenever the guest source changes.
//!
//! Set `SKIP_PVM_BUILDS=1` to emit an empty dummy blob instead (fast `cargo check`).

fn main() {
	pvm_builder::build_service(parasim_service::MANIFEST_DIR.as_ref());
}