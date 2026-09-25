//! Protocol constants shared by the service, the authorizer, and the PVF.

/// Runtime-side bound on a scheduled `:code` blob.
///
/// 16 MiB minus the 8-byte preimage encoding overhead, which fits the largest preimage the
/// vendored polkajam carries (`MAX_PREIMAGE_BLOB_LEN`, 32 MiB minus the same overhead).
/// Deliberately independent of the vendored fork — the value is a literal, not a re-export —
/// and pinned by [`tests::validation_code_fits_preimage_blob`].
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
