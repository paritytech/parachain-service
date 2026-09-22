//! Protocol constants shared by the service, the authorizer, and the PVF.

/// Runtime-side bound on a scheduled `:code` blob.
///
/// The largest preimage the network will ever carry once the vendored polkajam's
/// `MAX_PREIMAGE_LEN` is raised to 16 MiB: that encoded cap minus the 8-byte preimage
/// encoding overhead (`MAX_PREIMAGE_BLOB_LEN`). Deliberately independent of the vendored
/// fork — the value is a literal, not a re-export — and pinned by
/// [`tests::validation_code_fits_preimage_blob`].
pub const MAX_VALIDATION_CODE_SIZE: u32 = 16 * 1024 * 1024 - 8;

#[cfg(test)]
mod tests {
	use super::*;

	/// The scheduled `:code` blob must fit the largest preimage the network will carry.
	#[test]
	fn validation_code_fits_preimage_blob() {
		assert!(MAX_VALIDATION_CODE_SIZE as usize <= jam_types::MAX_PREIMAGE_BLOB_LEN);
	}
}
