//! Types shared between the parachain `service` and `authorizer` JAM programs.
//!
//! The two programs build into separate blobs and are peers — neither may depend
//! on the other's crate. Types they both need (starting with [`ParaId`]) live here
//! instead of being duplicated. This crate stays off the `sp-runtime`/`sp-core`
//! tree so it keeps compiling for PolkaVM; Polkadot's own `ParaId`
//! (`polkadot_parachain_primitives::Id`) is unusable here for that reason.

extern crate alloc;

use alloc::vec::Vec;
use codec::{Decode, Encode};

/// Unique identifier of a parachain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Encode, Decode)]
pub struct ParaId(pub u32);

impl From<u32> for ParaId {
	fn from(id: u32) -> Self {
		ParaId(id)
	}
}

impl From<ParaId> for u32 {
	fn from(id: ParaId) -> Self {
		id.0
	}
}

/// Upward messages emitted via host functions during Refine and replayed in order by Accumulate.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
pub enum UpwardMessage {
	SetKV { key: Vec<u8>, value: Vec<u8> },
}
