//! Parachain candidate payload: shared between the collator and the service host.

use codec::{Decode, Encode};

use crate::types::ValidationCodeHash;

/// Work-item payload for a parachain candidate (spec §3.2).
///
/// The host-side `service::refine` decodes it to look up the PVF by `validation_code_hash`.
///
/// The PoV travels as **work-item extrinsic 0**, not in this payload: JAM caps the first CE 133
/// message (the core index plus the work package, payload included) far below the size of a
/// multi-MiB PoV. The collator therefore declares the PoV with an `ExtrinsicSpec`, and the PVF
/// reads it back from work-item extrinsic 0 through `fetch`.
#[derive(Encode, Decode)]
pub struct ParachainCandidate {
	/// Hash of the currently active validation code. Refine uses this to look up
	/// the PVF bytecode from the preimage store.
	pub validation_code_hash: ValidationCodeHash,
}
