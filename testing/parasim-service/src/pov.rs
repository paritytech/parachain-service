//! Decode a parasim PoV into the para head, its claimed parent, and its anchor state proof.
//!
//! parasim accepts a work package without running a PVF, so the PoV is the same
//! `ParachainBlockData` a real collator produces (`cumulus_primitives_core`) and the parser here
//! must be a faithful `no_std` walker over its SCALE layout.
//!
//! Only `ParachainBlockData::V3` is accepted. V0/V1 carried no state proof, and without one there
//! is no way to know what the para's previous head was, which is exactly the check phase 4 exists
//! to add. Accepting them would leave a silent path back to the overwrite-anything behaviour.
//!
//! Wire shapes (verified against substrate/cumulus source):
//! - `V3` = `b"VERSIONEDPBD"` + byte `3` + `Vec<Block>` + `CompactProof` + `SchedulingProof`
//!   + `Vec<Option<AdditionalData>>`, where `AdditionalData = BTreeMap<String, Vec<u8>>`.
//! - `CompactProof { encoded_nodes: Vec<Vec<u8>> }` and `SchedulingProof` are both skipped: the
//!   former witnesses the parachain's own state (a radix-16 substrate trie, not JAM's), and the
//!   latter is relay-chain scheduling that carries no meaning on JAM. Collators send it empty.
//! - A cumulus collator zstd-compresses the PoV with an 8-byte magic prefix
//!   (`sp_maybe_compressed_blob`); parasim does not decompress, so such PoVs are rejected.
//! - Substrate `Block` = `Header + Vec<OpaqueExtrinsic>`; the para head is the **encoded header**
//!   alone, so the walker returns its original byte slice (byte-identical by construction, no
//!   re-encode drift).
//! - `Header<u32, BlakeTwo256>` = `parent_hash(32) + compact number + state_root(32) +
//!   extrinsics_root(32) + Digest`, and `Digest = Vec<DigestItem>` (compact length) where each
//!   item is tagged with a single wire byte holding the `DigestItemType` `#[repr(u32)]`
//!   discriminant: Other=0 (`Vec<u8>`), Consensus=4/Seal=5/PreRuntime=6 (`[u8;4]` + `Vec<u8>`),
//!   RuntimeEnvironmentUpdated=8 (unit).
//! - `SchedulingProof`'s relay `Header` has the same layout (its `number` is `#[codec(compact)]`
//!   too), so the same header walker handles it.

extern crate alloc;

use codec::{Compact, Decode};

/// Magic prefix a cumulus collator prepends to a zstd-compressed PoV
/// (`sp_maybe_compressed_blob::ZSTD_PREFIX`).
const ZSTD_PREFIX: [u8; 8] = [82, 188, 83, 118, 70, 219, 142, 5];
/// Literal prefix of a versioned `ParachainBlockData` (`VERSIONED_PARACHAIN_BLOCK_DATA_PREFIX`).
const VERSIONED_PREFIX: &[u8] = b"VERSIONEDPBD";
/// Version byte of `ParachainBlockData::V3`.
const V3_VERSION: u8 = 3;
/// `AdditionalData` key under which the collator puts the JAM anchor state proof.
pub const ANCHOR_STATE_PROOF_KEY: &str = "jam/anchor_state_proof";
/// Length of a substrate hash, and so of a header's leading `parent_hash`.
const HASH_LEN: usize = 32;

/// Why a PoV could not be turned into a para head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoVError {
	/// The PoV carries the zstd magic: parasim accepts no compressed PoVs
	/// (the collator must be configured to skip compression).
	Compressed,
	/// The PoV is not a parseable `ParachainBlockData::V3`.
	Malformed,
	/// A V3 PoV that carries no `jam/anchor_state_proof` entry. Without it the previous head
	/// cannot be established, so the package is unusable rather than merely unverified.
	MissingProof,
}

/// What refine needs out of a PoV.
#[derive(Debug, PartialEq, Eq)]
pub struct PoV<'a> {
	/// The last block's encoded header — the new para head.
	pub head: &'a [u8],
	/// The first block's `parent_hash`: the head this PoV claims to build on.
	pub parent_hash: [u8; HASH_LEN],
	/// The SCALE-encoded `(anchor_state_root, StateProof)` the collator attached.
	pub anchor_state_proof: &'a [u8],
}

/// Walk a V3 PoV, borrowing the head and proof out of it.
pub fn decode_pov(pov: &[u8]) -> Result<PoV<'_>, PoVError> {
	if pov.starts_with(&ZSTD_PREFIX) {
		return Err(PoVError::Compressed);
	}
	if !pov.starts_with(VERSIONED_PREFIX) {
		return Err(PoVError::Malformed);
	}

	let mut input = &pov[VERSIONED_PREFIX.len()..];
	if read_byte(&mut input)? != V3_VERSION {
		return Err(PoVError::Malformed);
	}

	let block_count = read_compact_len(&mut input)?;
	let mut first_parent_hash = None;
	let mut head = None;
	for _ in 0..block_count {
		let block = skip_block(&mut input)?;
		first_parent_hash.get_or_insert(block.parent_hash);
		head = Some(block.header);
	}
	let (Some(head), Some(parent_hash)) = (head, first_parent_hash) else {
		// A PoV with no blocks moves no head.
		return Err(PoVError::Malformed);
	};

	skip_compact_proof(&mut input)?;
	skip_scheduling_proof(&mut input)?;
	let anchor_state_proof = read_anchor_state_proof(&mut input)?;

	Ok(PoV { head, parent_hash, anchor_state_proof })
}

/// Find the `jam/anchor_state_proof` entry in `Vec<Option<AdditionalData>>`.
///
/// The slot is per block; the proof concerns the anchor rather than any one block, so the first
/// entry that carries it wins.
fn read_anchor_state_proof<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], PoVError> {
	let slots = read_compact_len(input)?;
	let mut found = None;
	for _ in 0..slots {
		if read_byte(input)? == 0 {
			// `None`: this block recorded no additional data.
			continue;
		}
		let entries = read_compact_len(input)?;
		for _ in 0..entries {
			let key = read_bytes(input)?;
			let value = read_bytes(input)?;
			if key == ANCHOR_STATE_PROOF_KEY.as_bytes() && found.is_none() {
				found = Some(value);
			}
		}
	}
	found.ok_or(PoVError::MissingProof)
}

/// Skip a `CompactProof { encoded_nodes: Vec<Vec<u8>> }`.
fn skip_compact_proof(input: &mut &[u8]) -> Result<(), PoVError> {
	let nodes = read_compact_len(input)?;
	for _ in 0..nodes {
		read_bytes(input)?;
	}
	Ok(())
}

/// Skip a `SchedulingProof { header_chain, internal_scheduling_parent_header,
/// signed_scheduling_info }`.
///
/// Its contents are relay-chain scheduling data with no meaning on JAM, so only its shape matters.
fn skip_scheduling_proof(input: &mut &[u8]) -> Result<(), PoVError> {
	let chain_len = read_compact_len(input)?;
	for _ in 0..chain_len {
		skip_header(input)?;
	}
	skip_header(input)?;
	// `signed_scheduling_info: Option<SignedSchedulingInfo>`; collators on JAM send `None`, and a
	// `Some` would carry relay-chain signatures parasim has no way to interpret.
	match read_byte(input)? {
		0 => Ok(()),
		_ => Err(PoVError::Malformed),
	}
}

/// A `u32` `Compact` length (the `Vec` prefix).
fn read_compact_len(input: &mut &[u8]) -> Result<u64, PoVError> {
	let len = Compact::<u32>::decode(input).map_err(|_| PoVError::Malformed)?;
	Ok(u64::from(u32::from(len)))
}

fn read_byte(input: &mut &[u8]) -> Result<u8, PoVError> {
	let (&byte, rest) = input.split_first().ok_or(PoVError::Malformed)?;
	*input = rest;
	Ok(byte)
}

/// Read a length-prefixed byte string (`Vec<u8>`, or a SCALE `String`).
fn read_bytes<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], PoVError> {
	let len = read_compact_len(input)? as usize;
	if input.len() < len {
		return Err(PoVError::Malformed);
	}
	let (bytes, rest) = input.split_at(len);
	*input = rest;
	Ok(bytes)
}

/// Take `len` octets off the front.
fn skip(input: &mut &[u8], len: usize) -> Result<(), PoVError> {
	if input.len() < len {
		return Err(PoVError::Malformed);
	}
	*input = &input[len..];
	Ok(())
}

/// Skip a substrate `Digest` (`Vec<DigestItem>`).
fn skip_digest(input: &mut &[u8]) -> Result<(), PoVError> {
	let count = read_compact_len(input)?;
	for _ in 0..count {
		match read_byte(input)? {
			// Other(Vec<u8>)
			0 => {
				read_bytes(input)?;
			},
			// Consensus / Seal / PreRuntime: ([u8; 4], Vec<u8>)
			4 | 5 | 6 => {
				skip(input, 4)?;
				read_bytes(input)?;
			},
			// RuntimeEnvironmentUpdated: unit
			8 => {},
			_ => return Err(PoVError::Malformed),
		}
	}
	Ok(())
}

/// One walked block: its encoded header and the parent it names.
struct Block<'a> {
	header: &'a [u8],
	parent_hash: [u8; HASH_LEN],
}

/// Walk one substrate `Block`, leaving `input` past the block's body.
fn skip_block<'a>(input: &mut &'a [u8]) -> Result<Block<'a>, PoVError> {
	let start = *input;
	skip_header(input)?;
	let header_len = start.len() - input.len();
	// `Vec<OpaqueExtrinsic>`: each extrinsic is its own length-prefixed blob.
	let extrinsics = read_compact_len(input)?;
	for _ in 0..extrinsics {
		read_bytes(input)?;
	}

	let header = &start[..header_len];
	if header.len() as u32 > parachain_service_interface::types::MAX_HEAD_DATA_SIZE {
		return Err(PoVError::Malformed);
	}
	let parent_hash =
		header[..HASH_LEN].try_into().expect("skip_header consumed at least a hash; qed");
	Ok(Block { header, parent_hash })
}

/// Skip a substrate `Header<u32, BlakeTwo256>`.
fn skip_header(input: &mut &[u8]) -> Result<(), PoVError> {
	// parent_hash
	skip(input, HASH_LEN)?;
	// number, compact-encoded
	read_compact_len(input)?;
	// state_root + extrinsics_root
	skip(input, 2 * HASH_LEN)?;
	skip_digest(input)
}

#[cfg(test)]
mod tests {
	use super::*;
	use alloc::{vec, vec::Vec};
	use codec::Encode;

	const EMPTY_DIGEST: &[u8] = &[0x00];

	/// Encode a substrate `Header<u32>`.
	fn header_bytes(parent: [u8; 32], number: u32, digest: &[u8]) -> Vec<u8> {
		let mut header = parent.to_vec();
		Compact::from(number).encode_to(&mut header);
		header.extend_from_slice(&[0u8; 32]); // state_root
		header.extend_from_slice(&[0u8; 32]); // extrinsics_root
		header.extend_from_slice(digest);
		header
	}

	/// Encode a substrate `Block`: header + `Vec<OpaqueExtrinsic>`.
	fn block_with(header: Vec<u8>, extrinsics: Vec<Vec<u8>>) -> Vec<u8> {
		let mut block = header;
		Compact::from(extrinsics.len() as u32).encode_to(&mut block);
		for extrinsic in extrinsics {
			extrinsic.encode_to(&mut block);
		}
		block
	}

	/// An empty `SchedulingProof`, exactly as upstream's `Default` encodes: no ancestry, a default
	/// relay header, no signed scheduling info.
	fn empty_scheduling_proof() -> Vec<u8> {
		let mut proof = vec![0x00]; // header_chain: empty
		proof.extend_from_slice(&header_bytes([0u8; 32], 0, EMPTY_DIGEST));
		proof.push(0x00); // signed_scheduling_info: None
		proof
	}

	/// Build a V3 PoV carrying `additional_data` entries for its single slot.
	fn v3_pov(blocks: Vec<Vec<u8>>, entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
		let mut pov = VERSIONED_PREFIX.to_vec();
		pov.push(V3_VERSION);
		Compact::from(blocks.len() as u32).encode_to(&mut pov);
		for block in &blocks {
			pov.extend_from_slice(block);
		}
		Compact::from(0u32).encode_to(&mut pov); // CompactProof: no nodes
		pov.extend_from_slice(&empty_scheduling_proof());

		// Vec<Option<AdditionalData>> with one Some(BTreeMap) slot.
		Compact::from(1u32).encode_to(&mut pov);
		pov.push(0x01);
		Compact::from(entries.len() as u32).encode_to(&mut pov);
		for (key, value) in entries {
			key.as_bytes().to_vec().encode_to(&mut pov);
			value.encode_to(&mut pov);
		}
		pov
	}

	fn proof_entry() -> Vec<(&'static str, Vec<u8>)> {
		vec![(ANCHOR_STATE_PROOF_KEY, vec![0xaa, 0xbb, 0xcc])]
	}

	#[test]
	fn single_block_works() {
		let header = header_bytes([7u8; 32], 1, EMPTY_DIGEST);
		let pov = v3_pov(vec![block_with(header.clone(), vec![])], proof_entry());

		let decoded = decode_pov(&pov).expect("valid V3 parses");
		assert_eq!(decoded.head, &header[..]);
		assert_eq!(decoded.parent_hash, [7u8; 32]);
		assert_eq!(decoded.anchor_state_proof, &[0xaa, 0xbb, 0xcc]);
	}

	/// With several bundled blocks the new head is the *last* one, but the ancestry check concerns
	/// the *first* block's parent — the bundle as a whole has to attach to the stored head.
	#[test]
	fn many_blocks_takes_last_head_and_first_parent_works() {
		let first = header_bytes([1u8; 32], 1, EMPTY_DIGEST);
		let last = header_bytes([2u8; 32], 2, EMPTY_DIGEST);
		let pov = v3_pov(
			vec![
				block_with(first, vec![vec![9u8; 3]]),
				block_with(last.clone(), vec![vec![8u8; 5]]),
			],
			proof_entry(),
		);

		let decoded = decode_pov(&pov).expect("valid V3 parses");
		assert_eq!(decoded.head, &last[..]);
		assert_eq!(decoded.parent_hash, [1u8; 32]);
	}

	/// Real headers carry seals and pre-runtime digests, so the walker must consume the tag bytes
	/// substrate actually emits rather than the SCALE declaration indices.
	#[test]
	fn real_digest_tags_works() {
		let mut digest = Vec::new();
		Compact::from(3u32).encode_to(&mut digest);
		digest.push(6); // PreRuntime
		digest.extend_from_slice(b"aura");
		vec![1u8; 8].encode_to(&mut digest);
		digest.push(8); // RuntimeEnvironmentUpdated
		digest.push(5); // Seal
		digest.extend_from_slice(b"aura");
		vec![2u8; 64].encode_to(&mut digest);

		let header = header_bytes([3u8; 32], 4, &digest);
		let pov = v3_pov(vec![block_with(header.clone(), vec![])], proof_entry());

		let decoded = decode_pov(&pov).expect("digest walker parses real tags");
		assert_eq!(decoded.head, &header[..]);
	}

	/// V0/V1 must not be accepted: they carry no state proof, so honouring them would restore the
	/// unverified overwrite path phase 4 removes.
	#[test]
	fn v1_pov_errors() {
		let mut pov = VERSIONED_PREFIX.to_vec();
		pov.push(1);
		Compact::from(1u32).encode_to(&mut pov);
		pov.extend_from_slice(&block_with(header_bytes([0u8; 32], 1, EMPTY_DIGEST), vec![]));
		Compact::from(0u32).encode_to(&mut pov);

		assert_eq!(decode_pov(&pov), Err(PoVError::Malformed));
	}

	/// A bare `Block` (legacy V0) has no version prefix at all.
	#[test]
	fn v0_pov_errors() {
		let v0 = block_with(header_bytes([0u8; 32], 1, EMPTY_DIGEST), vec![]);
		assert_eq!(decode_pov(&v0), Err(PoVError::Malformed));
	}

	/// A well-formed V3 that simply forgot the proof is reported distinctly, so the collator-side
	/// mistake is obvious in the logs rather than looking like a corrupt PoV.
	#[test]
	fn missing_proof_entry_errors() {
		let pov = v3_pov(
			vec![block_with(header_bytes([0u8; 32], 1, EMPTY_DIGEST), vec![])],
			vec![("polkadot/relay_proof", vec![1, 2, 3])],
		);
		assert_eq!(decode_pov(&pov), Err(PoVError::MissingProof));
	}

	#[test]
	fn compressed_pov_errors() {
		let mut pov = ZSTD_PREFIX.to_vec();
		pov.extend_from_slice(b"whatever");
		assert_eq!(decode_pov(&pov), Err(PoVError::Compressed));
	}

	#[test]
	fn malformed_pov_errors() {
		assert_eq!(decode_pov(&[]), Err(PoVError::Malformed));
		assert_eq!(decode_pov(b"VERSIONEDPBD"), Err(PoVError::Malformed));
		// Version byte present, but nothing after it.
		assert_eq!(decode_pov(b"VERSIONEDPBD\x03"), Err(PoVError::Malformed));
	}

	/// A PoV with no blocks moves no head, so it cannot be accepted.
	#[test]
	fn no_blocks_errors() {
		let pov = v3_pov(vec![], proof_entry());
		assert_eq!(decode_pov(&pov), Err(PoVError::Malformed));
	}

	/// Truncation anywhere in the PoV must be caught rather than read past.
	#[test]
	fn truncated_pov_errors() {
		let pov = v3_pov(
			vec![block_with(header_bytes([0u8; 32], 1, EMPTY_DIGEST), vec![])],
			proof_entry(),
		);
		for cut in 1..pov.len() {
			assert!(decode_pov(&pov[..cut]).is_err(), "truncated at {cut} must not parse");
		}
	}
}
