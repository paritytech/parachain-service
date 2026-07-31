#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! Shared support code for the parachain `service` and `authorizer` JAM programs.
//!
//! They build into separate blobs and are peers — neither may depend on the
//! other's crate — so anything they both need lives here. Kept off the
//! `sp-runtime`/`sp-core` tree so it stays buildable for PolkaVM.

pub mod types;
