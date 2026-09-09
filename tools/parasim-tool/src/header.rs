//! Decoding the substrate header that parasim carries as a para head.

use codec::Decode as _;

/// Substrate hash length, and so the size of a header's leading `parent_hash`.
pub const HASH_LEN: usize = 32;

/// The fields of a substrate `Header` worth showing.
pub struct Header {
	pub parent_hash: [u8; HASH_LEN],
	pub number: u32,
	pub state_root: [u8; HASH_LEN],
}

/// `Header` = parent_hash(32) ++ compact number ++ state_root(32) ++ extrinsics_root(32) ++ digest.
pub fn decode(head: &[u8]) -> Result<Header, String> {
	let mut rest = head;
	let parent_hash = take_hash(&mut rest)?;
	let codec::Compact(number) =
		codec::Compact::<u32>::decode(&mut rest).map_err(|e| format!("block number: {e}"))?;
	let state_root = take_hash(&mut rest)?;
	Ok(Header { parent_hash, number, state_root })
}

fn take_hash(input: &mut &[u8]) -> Result<[u8; HASH_LEN], String> {
	if input.len() < HASH_LEN {
		return Err("truncated".into());
	}
	let (hash, rest) = input.split_at(HASH_LEN);
	*input = rest;
	Ok(hash.try_into().expect("split at HASH_LEN; qed"))
}
