//! Shared AURA protocol and the local development sentinel.

pub use jp_aura_authorizer::*;

/// The collator key that means "admit this package without any of the checks a collator has to
/// pass": no signature, and no para assigned to the core.
///
/// It is the only way a core-assignment command reaches a *parked* core, whose config names no
/// para and therefore admits no work item at all — including the one carrying the command that
/// would un-park it.
///
/// The escalation rides the key rather than a field of its own so that [`AuthToken`] stays
/// exactly the design's three fields: a collator that has never heard of this encodes the same
/// bytes it always did.
///
/// All-ones is no key's encoding — the low 255 bits of a compressed point are a coordinate
/// reduced modulo the two curves' shared field prime, and all-ones is not reduced — so no keygen
/// can emit it, no collator set can hold it, and no honest token can collide with it. The
/// contract tests pin that.
///
/// FIXME: a dev-network debugging bypass. It widens command access to anyone, on any core this
/// authorizer guards. Accepted for the POC (user decision, 2026-09-02); it must not ship.
pub const SUDO_KEY: CollatorKey = [0xFF; 32];
