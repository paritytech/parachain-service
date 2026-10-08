//! Shared AURA protocol and the local development sentinel.

pub use jp_aura_authorizer::*;

/// Development sentinel that bypasses signature, membership, item-count, and slot checks.
/// Target-service validation still applies. This permits control commands on parked cores
/// whose config has no para IDs. The sentinel preserves the three-field token encoding.
///
/// FIXME: Anyone can submit unsigned control commands with this key. Remove or isolate
/// this bypass before production use.
pub const SUDO_KEY: CollatorKey = [0xFF; 32];
