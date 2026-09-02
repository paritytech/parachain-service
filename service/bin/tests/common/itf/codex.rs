use std::collections::BTreeMap;

use jam_std_common::hash_raw;
use jam_types::{AuthTrace, AuthorizerHash, Hash};
use parachain_service::work_digest::{ValidationCodeHash, ValidationCodeRef};
use parachain_service_bin::BLOB as SERVICE;
use parachain_service_interface::types::{HeadData, ParaId};

/// Deterministic mapping between Quint's abstract integers and Rust values.
#[derive(Debug)]
pub struct Codex {
	lengths: BTreeMap<i128, u32>,
	hashes: BTreeMap<i128, Hash>,
}

impl Default for Codex {
	fn default() -> Self {
		let mut codex = Self { lengths: BTreeMap::new(), hashes: BTreeMap::new() };
		codex.hashes.insert(0, hash_raw(SERVICE));
		codex
	}
}

impl Codex {
	pub fn para_id(value: i128) -> Result<ParaId, String> {
		Ok(ParaId(checked_u32(value, "ParaId")?))
	}

	pub fn para_int(value: ParaId) -> i128 {
		value.0.into()
	}

	pub fn head(value: i128) -> Result<HeadData, String> {
		let value = u64::try_from(value).map_err(|_| format!("HeadData out of range: {value}"))?;
		if value == 0 {
			return Ok(HeadData::default());
		}
		value
			.to_le_bytes()
			.to_vec()
			.try_into()
			.map_err(|_| "encoded HeadData exceeds cap".into())
	}

	pub fn head_int(value: &HeadData) -> Result<i128, String> {
		if value.is_empty() {
			return Ok(0);
		}
		let bytes: [u8; 8] = value
			.as_slice()
			.try_into()
			.map_err(|_| format!("codex HeadData must be 8 bytes, got {}", value.len()))?;
		Ok(u64::from_le_bytes(bytes).into())
	}

	pub fn validation_code(&mut self, value: i128, len: i128) -> Result<ValidationCodeRef, String> {
		let len = checked_u32(len, "validation code length")?;
		Ok(ValidationCodeRef { hash: ValidationCodeHash(self.hash(value, len)?), len })
	}

	pub fn hash(&mut self, value: i128, len: u32) -> Result<Hash, String> {
		if let Some(previous) = self.lengths.get(&value) {
			if *previous != len {
				return Err(format!(
					"abstract hash {value} used with conflicting lengths {previous} and {len}"
				));
			}
		} else {
			self.lengths.insert(value, len);
		}
		if let Some(hash) = self.hashes.get(&value) {
			return Ok(*hash);
		}
		let hash = hash_raw(&Self::blob(value, len)?);
		self.hashes.insert(value, hash);
		Ok(hash)
	}

	pub fn hash_int(&self, hash: Hash) -> Result<i128, String> {
		self.hashes
			.iter()
			.find_map(|(value, known)| (*known == hash).then_some(*value))
			.ok_or_else(|| format!("hash is not registered in the Quint codex: {hash:?}"))
	}

	pub fn blob(value: i128, len: u32) -> Result<Vec<u8>, String> {
		let value = u64::try_from(value).map_err(|_| format!("hashBytes out of range: {value}"))?;
		let mut blob = vec![0; len as usize];
		let encoded = value.to_le_bytes();
		let copied = blob.len().min(encoded.len());
		blob[..copied].copy_from_slice(&encoded[..copied]);
		Ok(blob)
	}

	pub fn authorizer_hash(value: i128) -> Result<AuthorizerHash, String> {
		let mut bytes = [0; 32];
		bytes[..4].copy_from_slice(&checked_u32(value, "authorizer hash")?.to_le_bytes());
		Ok(AuthorizerHash(bytes))
	}

	pub fn authorizer_int(value: AuthorizerHash) -> Result<i128, String> {
		if value.0[4..] != [0; 28] {
			return Err("authorizer hash is not a codex value".into());
		}
		Ok(u32::from_le_bytes(value.0[..4].try_into().unwrap()).into())
	}

	pub fn auth_trace(len: i128) -> Result<AuthTrace, String> {
		let len =
			usize::try_from(len).map_err(|_| format!("auth trace length out of range: {len}"))?;
		if len > 256 {
			return Err(format!("auth trace length exceeds 256: {len}"));
		}
		Ok(AuthTrace(vec![0xaa; len]))
	}

	pub fn auth_trace_int(value: &AuthTrace) -> Result<i128, String> {
		if value.0.iter().any(|byte| *byte != 0xaa) {
			return Err("auth trace is not a codex value".into());
		}
		Ok(value.0.len() as i128)
	}
}

fn checked_u32(value: i128, name: &str) -> Result<u32, String> {
	u32::try_from(value).map_err(|_| format!("{name} out of range: {value}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn head_round_trip_works() {
		for value in [0, 1, i64::MAX as i128] {
			let head = Codex::head(value).unwrap();
			assert_eq!(Codex::head_int(&head).unwrap(), value);
		}
	}

	#[test]
	fn same_hash_and_length_works() {
		let mut codex = Codex::default();
		assert_eq!(codex.hash(7, 16).unwrap(), codex.hash(7, 16).unwrap());
	}

	#[test]
	fn conflicting_hash_length_errors() {
		let mut codex = Codex::default();
		let original = codex.hash(7, 16).unwrap();
		let error = codex.hash(7, 17).unwrap_err();
		assert!(error.contains("conflicting lengths 16 and 17"));
		assert_eq!(codex.hash(7, 16).unwrap(), original);
	}

	#[test]
	fn mapped_values_round_trip_works() {
		let mut codex = Codex::default();
		let para = Codex::para_id(23).unwrap();
		assert_eq!(Codex::para_int(para), 23);

		let hash = codex.hash(91, 32).unwrap();
		assert_eq!(codex.hash_int(hash).unwrap(), 91);

		let authorizer = Codex::authorizer_hash(1234).unwrap();
		assert_eq!(Codex::authorizer_int(authorizer).unwrap(), 1234);

		let trace = Codex::auth_trace(256).unwrap();
		assert_eq!(Codex::auth_trace_int(&trace).unwrap(), 256);
	}

	#[test]
	fn service_hash_zero_works() {
		let mut codex = Codex::default();
		assert_eq!(codex.hash(0, SERVICE.len() as u32).unwrap(), hash_raw(SERVICE));
	}

	#[test]
	fn auth_trace_over_cap_errors() {
		assert!(Codex::auth_trace(257).unwrap_err().contains("exceeds 256"));
	}
}
