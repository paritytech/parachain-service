//! Shared formatting helpers.

use std::fmt::Write as _;

/// Lower-case hex, no `0x` prefix.
pub fn hex(bytes: &[u8]) -> String {
	bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
		let _ = write!(out, "{byte:02x}");
		out
	})
}
