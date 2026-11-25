//! Protocol constants shared by the service, the authorizer, and the PVF.

/// Maximum size of a scheduled `:code` blob (16 MiB minus 8-byte preimage overhead).
pub const MAX_VALIDATION_CODE_SIZE: u32 = 16 * 1024 * 1024 - 8;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn validation_code_fits_preimage_blob_works() {
		assert!(MAX_VALIDATION_CODE_SIZE as usize <= jam_types::MAX_PREIMAGE_BLOB_LEN);
	}
}
