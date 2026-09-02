#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! The ed25519 parachain-service authorizer blob.
//!
//! One of the two verifier blobs over `parachain-authorizer`'s scheme-blind core: the pipeline,
//! the wire types and the collator trie all live there, and this crate is the signature check
//! plus the PVM entry point. Pick it for a runtime whose `AuraId` is ed25519 — Asset Hub Polkadot
//! and the other chains carrying the 2022 Shell hotfix.

use jam_types::{AuthTrace, CoreIndex};
use parachain_authorizer::aura::{CollatorKey, CollatorSignature, SignatureScheme};

/// Bump the PVM guest stack for `curve25519-dalek`'s serial-64 `verify_strict`
/// (its Straus double-scalar tables exceed polkavm's default 8 KiB stack).
/// No-op on host builds (the `polkavm_derive` macro is only defined on riscv+e).
macro_rules! min_stack_size {
	($size:expr) => {
		#[cfg(all(any(target_arch = "riscv32", target_arch = "riscv64"), target_feature = "e"))]
		polkavm_derive::min_stack_size!($size);
	};
}
min_stack_size!(32768); // 32 KiB guest stack.

/// Directory of this crate's `Cargo.toml`, used by `parachain-authorizer-ed25519-bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// ed25519 collator signatures, as `sp_core::ed25519` produces them.
pub struct Ed25519;

impl SignatureScheme for Ed25519 {
	/// Uses `verify_strict` (not `verify`) to reject cofactored/non-canonical signatures and
	/// low-order public keys — required for deterministic validator agreement across
	/// implementations.
	fn verify(key: &CollatorKey, signature: &CollatorSignature, payload: &[u8]) -> bool {
		let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(key) else { return false };
		key.verify_strict(payload, &ed25519_dalek::Signature::from_bytes(signature)).is_ok()
	}
}

pub struct ParachainAuthorizer;
jam_pvm_common::declare_authorizer!(ParachainAuthorizer);

impl jam_pvm_common::Authorizer for ParachainAuthorizer {
	fn is_authorized(core: CoreIndex) -> AuthTrace {
		parachain_authorizer::authorize::<Ed25519>(core)
	}
}
