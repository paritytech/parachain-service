//! Deriving a 31-octet JAM state key from a service's own storage key.
//!
//! A service addresses its storage by a key of its own choosing, but that is not the key the
//! state trie is built over. JAM interleaves the service id with a hash of the service-local key
//! so that one service's entries cannot collide with another's. Three parties must agree on this
//! mapping byte-for-byte — the collator asking for a proof, the guest verifying it, and the node
//! serving it — which is why it lives here rather than being open-coded at each site.
//!
//! Mirrors the `ServiceKey::Value` arm of `impl From<ServiceKey> for StorageKey` in
//! `jam-std-common`; the test module pins the two together.

use crate::{blake2_256, StateKey};

/// Marks a key as addressing a service's *storage*, as opposed to its metadata or preimages.
const VALUE_PREFIX: [u8; 4] = [0xff, 0xff, 0xff, 0xff];

/// The state key of `service_id`'s storage entry under its own `key`.
pub fn service_value_state_key(service_id: u32, key: &[u8]) -> StateKey {
	let hash = blake2_256(&[VALUE_PREFIX.as_slice(), key].concat());

	let id = service_id.to_le_bytes();
	let mut state_key = [0u8; 31];
	// The first eight octets interleave the service id with the hash, so that the entries of one
	// service are spread across the trie rather than sharing a common prefix.
	state_key[..8]
		.copy_from_slice(&[id[0], hash[0], id[1], hash[1], id[2], hash[2], id[3], hash[3]]);
	state_key[8..].copy_from_slice(&hash[4..27]);
	state_key
}

/// The state key of `service_id`'s preimage request for a value of length `len` and hash `hash`.
///
/// Unlike a service's own storage, the service does not choose this key: JAM derives it from the
/// length and hash of the preimage, so any party can compute where a request lands from the
/// preimage alone. Mirrors the `ServiceKey::Request` arm of `jam-std-common` — note the preimage
/// is `len.to_le_bytes() ‖ hash` with no prefix (the `0xfe 0xff 0xff 0xff` prefix belongs to the
/// `Preimage` arm).
pub fn service_request_state_key(service_id: u32, hash: &[u8; 32], len: u32) -> StateKey {
	let hash = blake2_256(&[&len.to_le_bytes()[..], &hash[..]].concat());

	let id = service_id.to_le_bytes();
	let mut state_key = [0u8; 31];
	// Same interleave as the `Value` arm: the service id is woven through the hash so that the
	// entries of one service are spread across the trie rather than sharing a common prefix.
	state_key[..8]
		.copy_from_slice(&[id[0], hash[0], id[1], hash[1], id[2], hash[2], id[3], hash[3]]);
	state_key[8..].copy_from_slice(&hash[4..27]);
	state_key
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The whole point of this module: our derivation must be byte-identical to polkajam's, or the
	/// guest would verify a proof for a key the node never proved.
	#[test]
	fn matches_jam_std_common() {
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
	}

	/// Distinct paras must land on distinct keys; a shared prefix would be a silent aliasing bug.
	#[test]
	fn distinct_keys_for_distinct_paras() {
		let para_0 = service_value_state_key(9, &[0x00, 0, 0, 0, 0]);
		let para_1 = service_value_state_key(9, &[0x00, 1, 0, 0, 0]);
		assert_ne!(para_0, para_1);
	}

	/// ...and so must the same key under different services.
	#[test]
	fn distinct_keys_for_distinct_services() {
		let key = [0x00, 0, 0, 0, 0];
		assert_ne!(service_value_state_key(9, &key), service_value_state_key(10, &key));
	}
}
