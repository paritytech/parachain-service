//! Derivation of 31-byte JAM state keys from service-local storage and preimage keys.
//!
//! Mirrors `impl From<ServiceKey> for StorageKey` in `jam-std-common`.

use crate::{blake2_256, types::ServiceId, StateKey};

const VALUE_PREFIX: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const PREIMAGE_PREFIX: [u8; 4] = [0xfe, 0xff, 0xff, 0xff];

fn interleave_service_key(service_id: ServiceId, hash: [u8; 32]) -> StateKey {
	let [id0, id1, id2, id3] = service_id.to_le_bytes();
	let [h0, h1, h2, h3, ..] = hash;

	let mut key = [0; 31];
	key[..8].copy_from_slice(&[id0, h0, id1, h1, id2, h2, id3, h3]);
	key[8..].copy_from_slice(&hash[4..27]);
	key
}

/// State key for a service storage value (`ServiceKey::Value`).
pub fn service_value_state_key(service_id: ServiceId, key: &[u8]) -> StateKey {
	let hash = blake2_256(&[VALUE_PREFIX.as_slice(), key].concat());
	interleave_service_key(service_id, hash)
}

/// State key for a service preimage (`ServiceKey::Preimage`).
pub fn service_preimage_state_key(service_id: ServiceId, hash: &[u8; 32]) -> StateKey {
	let hash = blake2_256(&[PREIMAGE_PREFIX.as_slice(), &hash[..]].concat());
	interleave_service_key(service_id, hash)
}

/// State key for a service preimage request (`ServiceKey::Request`).
pub fn service_request_state_key(service_id: ServiceId, hash: &[u8; 32], len: u32) -> StateKey {
	let hash = blake2_256(&[&len.to_le_bytes()[..], &hash[..]].concat());
	interleave_service_key(service_id, hash)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn matches_jam_std_common_works() {
		let cases: &[(u32, &[u8])] = &[
			(0, &[]),
			(0, &[0x00, 0x00, 0x00, 0x00, 0x00]),
			(1, &[0x00]),
			(5, &[0x00, 0x03, 0x00, 0x00, 0x00]),
			(0xffff_ffff, &[0xff; 64]),
			(0x0100_0001, b"a longer service-local key than one hash block would hold"),
		];

		for (service_id, key) in cases {
			let ours = service_value_state_key(*service_id, key);
			let theirs: jam_std_common::StorageKey =
				jam_std_common::ServiceKey::Value { id: *service_id, key }.into();
			assert_eq!(ours, *theirs, "service {service_id}, key {key:02x?}");
		}

		let request_cases: &[(u32, u32, [u8; 32])] = &[
			(0, 0, [0x00; 32]),
			(1, u32::MAX, [0xff; 32]),
			(5, 4 * 1024 * 1024, [0xab; 32]),
			(0xffff_ffff, 4 * 1024 * 1024, [0x00; 32]),
			(0x0100_0001, 0, [0xff; 32]),
		];

		for (service_id, len, hash) in request_cases {
			let ours = service_request_state_key(*service_id, hash, *len);
			let theirs: jam_std_common::StorageKey =
				jam_std_common::ServiceKey::Request { id: *service_id, len: *len, hash: *hash }
					.into();
			assert_eq!(ours, *theirs, "service {service_id}, len {len}, hash {hash:02x?}");
		}

		for (service_id, hash) in [(0, [0x00; 32]), (1, [0xff; 32]), (0x0100_0001, [0xab; 32])] {
			let ours = service_preimage_state_key(service_id, &hash);
			let theirs: jam_std_common::StorageKey =
				jam_std_common::ServiceKey::Preimage { id: service_id, hash }.into();
			assert_eq!(ours, *theirs, "service {service_id}, hash {hash:02x?}");
		}
	}

	#[test]
	fn distinct_paras_works() {
		let para_0 = service_value_state_key(9, &[0x00, 0, 0, 0, 0]);
		let para_1 = service_value_state_key(9, &[0x00, 1, 0, 0, 0]);
		assert_ne!(para_0, para_1);
	}

	#[test]
	fn distinct_services_works() {
		let key = [0x00, 0, 0, 0, 0];
		assert_ne!(service_value_state_key(9, &key), service_value_state_key(10, &key));
	}
}
