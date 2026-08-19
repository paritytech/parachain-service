//! Parachain candidate payload: shared between the PVF guest and the service host.

extern crate alloc;

use alloc::vec::Vec;
use codec::{Decode, Encode};

use crate::types::ValidationCodeHash;

// TODO: No requirement to have this type here. Parachain service does not need to provide this
// type.
/// Work-item payload for a parachain candidate.
///
/// Decoded from the raw work-item payload bytes by both:
/// - the host-side `service::refine` (to look up the PVF by `validation_code_hash`), and
/// - the guest PVF (to obtain the PoV via the `work_item_payload(0)` host call).
#[derive(Encode, Decode)]
pub struct ParachainCandidate {
	/// Hash of the currently active validation code. Refine uses this to look up
	/// the PVF bytecode from the preimage store.
	pub validation_code_hash: ValidationCodeHash,
	/// Proof-of-Validity — the block data and witness the PVF validates.
	pub pov: Vec<u8>,
}
