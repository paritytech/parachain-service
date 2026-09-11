//! Map Quint's `RefineLog` to the Rust [`RefineLog`] (§3.3).

use parachain_service::work_digest::{RefineLog, MAX_REPORT_ERROR_PAYLOAD};
use serde_json::Value;

use super::replay::{integer, variant};

/// Parse a Quint `RefineLog` into the Rust [`RefineLog`].
///
/// The Rust service can only ever log refine errors it can produce itself, so
/// variants the model uses for package-level failures that the Rust `refine`
/// panics on (§4.1 step 1, §4.2) are rejected: they surface to Accumulate as a
/// gray-paper work error, not as a logged `RefineLogEntry`.
pub fn refine_log(value: &Value) -> Result<RefineLog, String> {
	let (tag, value) = variant(value)?;
	match tag {
		"InvalidCodeHash" => Ok(RefineLog::InvalidCodeHash),
		"Opaque" => Ok(RefineLog::Opaque(
			opaque_payload(integer(value)?)?
				.try_into()
				.map_err(|_| "Opaque payload exceeds 1024 bytes".to_string())?,
		)),
		"SetValidatorKeysTooManyKeys" => Ok(RefineLog::TooManyValidatorKeys),
		"TooManyUpwardMessages" => Ok(RefineLog::TooManyUpwardMessages),
		"RestrictedHostFunction" => Ok(RefineLog::RestrictedHostFunction),
		"RefineOutputTooLarge" => Ok(RefineLog::RefineOutputTooLarge),
		"MissingHeadDeclaration" => Ok(RefineLog::MissingHeadDeclaration),
		// `is_authorized` runs before Refine and rejects an undecodable config
		// (`UndecodableAuthConfig`) and a config naming a different number of
		// paras than items (`InvalidWorkItemCount`), so these can never reach
		// Refine as a logged `RefineLogEntry`. A multi-item package — which
		// Refine's single-item restriction (§3.2) panics on — never yields a
		// logged `RefineLogEntry` either.
		"MalformedAuthorizerConfig" | "AuthConfigMismatch" => Err(format!(
			"{tag} is rejected by `is_authorized` before Refine runs, so it never surfaces as a \
			 logged RefineLogEntry"
		)),
		"InvalidItemCount" => Err(format!(
			"InvalidItemCount is a package-level Refine failure; the Rust service panics on it \
			 (§3.2 single-item restriction), so it surfaces as a gray-paper work error, not a \
			 logged RefineLogEntry"
		)),
		other => Err(format!("unsupported RefineLog variant {other}")),
	}
}

/// The model's `Opaque` payload is an abstract byte-length proxy. Materialize a
/// deterministic payload of that length so Rust's exact SCALE-size accounting
/// agrees with the model.
fn opaque_payload(value: i128) -> Result<Vec<u8>, String> {
	let len =
		u32::try_from(value).map_err(|_| format!("Opaque payload length out of range: {value}"))?;
	if len > MAX_REPORT_ERROR_PAYLOAD {
		return Err(format!("Opaque payload exceeds {MAX_REPORT_ERROR_PAYLOAD} bytes: {len}"));
	}
	Ok(vec![0xaa; len as usize])
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn nullary_variants_work() {
		for (tag, expected) in [
			("InvalidCodeHash", RefineLog::InvalidCodeHash),
			("SetValidatorKeysTooManyKeys", RefineLog::TooManyValidatorKeys),
			("TooManyUpwardMessages", RefineLog::TooManyUpwardMessages),
			("RestrictedHostFunction", RefineLog::RestrictedHostFunction),
			("RefineOutputTooLarge", RefineLog::RefineOutputTooLarge),
			("MissingHeadDeclaration", RefineLog::MissingHeadDeclaration),
		] {
			let value = json!({ "tag": tag, "value": { "#tup": [] } });
			assert_eq!(refine_log(&value).unwrap(), expected, "{tag}");
		}
	}

	#[test]
	fn opaque_zero_works() {
		let value = json!({ "tag": "Opaque", "value": { "#bigint": "0" } });
		assert_eq!(
			refine_log(&value).unwrap(),
			RefineLog::Opaque(Vec::<u8>::new().try_into().unwrap())
		);
	}

	#[test]
	fn opaque_length_proxy_works() {
		let value = json!({ "tag": "Opaque", "value": { "#bigint": "42" } });
		assert_eq!(
			refine_log(&value).unwrap(),
			RefineLog::Opaque(vec![0xaa; 42].try_into().unwrap())
		);
	}

	#[test]
	fn opaque_invalid_lengths_error() {
		for value in [-1, i128::from(MAX_REPORT_ERROR_PAYLOAD) + 1] {
			let value = json!({ "tag": "Opaque", "value": { "#bigint": value.to_string() } });
			assert!(refine_log(&value).unwrap_err().contains("Opaque payload"));
		}
	}

	#[test]
	fn package_level_failures_errors() {
		for (tag, needle) in [
			("MalformedAuthorizerConfig", "is_authorized"),
			("AuthConfigMismatch", "is_authorized"),
			("InvalidItemCount", "panics"),
		] {
			let value = json!({ "tag": tag, "value": { "#tup": [] } });
			let error = refine_log(&value).unwrap_err();
			assert!(error.contains(tag), "{tag}");
			assert!(error.contains(needle), "{tag}: {error}");
		}
	}

	#[test]
	fn unknown_variant_errors() {
		let value = json!({ "tag": "Surprise", "value": { "#tup": [] } });
		assert!(refine_log(&value).unwrap_err().contains("Surprise"));
	}
}
