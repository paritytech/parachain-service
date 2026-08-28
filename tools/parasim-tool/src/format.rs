//! Shared formatting and parsing helpers.

use std::fmt::Write as _;

/// Lower-case hex, no `0x` prefix.
pub fn hex(bytes: &[u8]) -> String {
	bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
		let _ = write!(out, "{byte:02x}");
		out
	})
}

/// Parse a `0x`-prefixed or bare 32-byte hex header hash.
pub fn parse_header_hash(text: &str) -> Result<jam_interface::HeaderHash, String> {
	use crate::header::HASH_LEN;
	let text = text.strip_prefix("0x").unwrap_or(text);
	if text.len() != HASH_LEN * 2 {
		return Err(format!("expected a {}-hex-digit block hash, got {}", HASH_LEN * 2, text.len()));
	}
	let mut hash = [0u8; HASH_LEN];
	for (index, byte) in hash.iter_mut().enumerate() {
		*byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
			.map_err(|e| format!("bad hex in block hash: {e}"))?;
	}
	Ok(hash.into())
}
