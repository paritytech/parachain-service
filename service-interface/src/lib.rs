//! Shared interface code of the parachain service. Used by `pvf`s and `authorizer`s.

#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

pub mod candidate;
pub mod host_call;
pub mod types;
pub mod upward_message;
