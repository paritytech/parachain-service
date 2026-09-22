#![cfg_attr(any(target_arch = "riscv32", target_arch = "riscv64"), no_std)]

//! The sr25519 parachain-service authorizer blob.
//!
//! One of the two verifier blobs over `parachain-authorizer`'s scheme-blind core: the pipeline,
//! the wire types and the collator trie all live there, and this crate is the signature check
//! plus the PVM entry point. Pick it for a runtime whose `AuraId` is sr25519 — the default, and
//! what the parachain template uses.
//!
//! Raw `schnorrkel` rather than `sp-core`: the two verify identically (`sp_core::sr25519::Pair::
//! verify` is this function), and sp-core costs 2.4 kB of a 64,000-byte ceiling for it.
//! `authorizer-sr25519-experiment/README.md` has both measurements.

use jam_types::{AuthTrace, CoreIndex};
use parachain_authorizer::aura::{CollatorKey, CollatorSignature, SignatureScheme};

/// Bump the PVM guest stack for `curve25519-dalek`'s serial-64 double-scalar multiplication
/// (its Straus tables exceed polkavm's default 8 KiB stack).
/// No-op on host builds (the `polkavm_derive` macro is only defined on riscv+e).
macro_rules! min_stack_size {
	($size:expr) => {
		#[cfg(all(any(target_arch = "riscv32", target_arch = "riscv64"), target_feature = "e"))]
		polkavm_derive::min_stack_size!($size);
	};
}
min_stack_size!(32768); // 32 KiB guest stack.

/// Directory of this crate's `Cargo.toml`, used by `parachain-authorizer-sr25519-bin`'s
/// `build.rs` to locate the crate when compiling it into a PVM blob.
pub const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// The transcript context every sr25519 signature in Substrate is made under.
///
/// Not ours to choose: a collator signs through `Keystore::sr25519_sign`, which goes to
/// `sp_core::sr25519::Pair::sign` and its hard-coded `SIGNING_CTX`, with no way to pass another.
/// Verifying under any other context fails every signature, and fails it the same way a wrong key
/// does. Domain separation for this protocol comes from `WORK_PACKAGE_SIGN_CTX` inside the signed
/// payload, so nothing is lost by sharing the context with the rest of Substrate.
const SIGNING_CONTEXT: &[u8] = b"substrate";

/// sr25519 collator signatures, as `sp_core::sr25519` produces them.
pub struct Sr25519;

impl SignatureScheme for Sr25519 {
	/// There is no `verify_strict` equivalent here and none is needed: ristretto is a prime-order
	/// group with a canonical encoding, so `PublicKey::from_bytes` already rejects what
	/// `verify_strict` exists to reject.
	fn verify(key: &CollatorKey, signature: &CollatorSignature, payload: &[u8]) -> bool {
		let (Ok(key), Ok(signature)) =
			(schnorrkel::PublicKey::from_bytes(key), schnorrkel::Signature::from_bytes(signature))
		else {
			return false;
		};
		key.verify_simple(SIGNING_CONTEXT, payload, &signature).is_ok()
	}
}

pub struct ParachainAuthorizer;
jam_pvm_common::declare_authorizer!(ParachainAuthorizer);

impl jam_pvm_common::Authorizer for ParachainAuthorizer {
	fn is_authorized(core: CoreIndex) -> AuthTrace {
		parachain_authorizer::authorize::<Sr25519>(core)
	}
}
