//! Decode a parasim PoV into the para `HeadData`.
//!
//! parasim accepts a work package without running a PVF. The PoV is the same
//! `ParachainBlockData` a real collator produces (`cumulus_primitives_core`),
//! so the parser here must be a faithful, no_std walker over its SCALE layout to
//! find the last block's encoded header — the byte slice that a real PVF would
//! hand to `set_head`.
//!
//! Wire shapes (verified against substrate/cumulus source):
//! - `ParachainBlockData::V0` (legacy): `Block + CompactProof`, no magic prefix.
//! - `ParachainBlockData::V1`: literal `b"VERSIONEDPBD"` + byte `1` +
//!   `Vec<Block>` + `CompactProof { encoded_nodes: Vec<Vec<u8>> }`.
//! - A cumulus collator zstd-compresses the PoV with an 8-byte magic prefix
//!   (`sp_maybe_compressed_blob`, `[82,188,83,118,70,219,142,5]`); parasim does
//!   not decompress, so such PoVs are rejected with a clear error.
//! - Substrate `Block` = `Header + Vec<OpaqueExtrinsic>`; the para head is the
//!   **encoded header** alone, so the walker returns its original byte slice
//!   (byte-identical by construction, no re-encode drift).
//! - `Header<u32, BlakeTwo256>` = `parent_hash(32) + compact number +
//!   state_root(32) + extrinsics_root(32) + Digest`. `Digest` = `Vec<DigestItem>`
//!   (compact length); each item is tagged with a **single wire byte** — the
//!   SCALE variant index of `DigestItemType` in *declaration* order, not the
//!   `#[repr(u32)]` values. Wire tags: Other=0 (`Vec<u8>`), Consensus/Seal/
//!   PreRuntime=1/2/3 (`[u8;4]` + `Vec<u8>`), RuntimeEnvironmentUpdated=4 (unit).
//!   (The `#[repr(u32)]` values 0/4/5/6/8 never appear on the wire.)

extern crate alloc;

use alloc::vec::Vec;
use codec::{Compact, Decode};

/// Magic prefix a cumulus collator prepends to a zstd-compressed PoV
/// (`sp_maybe_compressed_blob::ZSTD_PREFIX`).
const ZSTD_PREFIX: [u8; 8] = [82, 188, 83, 118, 70, 219, 142, 5];
/// Literal prefix of `ParachainBlockData::V1` (`VERSIONED_PARACHAIN_BLOCK_DATA_PREFIX`).
const VERSIONED_PREFIX: &[u8] = b"VERSIONEDPBD";
/// Version byte of `ParachainBlockData::V1`.
const V1_VERSION: u8 = 1;

/// Why a PoV could not be turned into a para head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoVError {
	/// The PoV carries the zstd magic: parasim accepts no compressed PoVs
	/// (the collator must be configured to skip compression).
	Compressed,
	/// The PoV is neither a parseable `ParachainBlockData` nor a parseable
	/// header.
	Malformed,
}

/// The byte slice of the last block's encoded header — the para head.
///
/// Bounded by `MAX_HEAD_DATA_SIZE` by construction (`set_head` rejects anything
/// larger).
pub fn decode_para_head(pov: &[u8]) -> Result<Vec<u8>, PoVError> {
	if pov.starts_with(&ZSTD_PREFIX) {
		return Err(PoVError::Compressed);
	}

	if pov.starts_with(VERSIONED_PREFIX) {
		let mut input = &pov[VERSIONED_PREFIX.len()..];
		if read_byte(&mut input)? != V1_VERSION {
			return Err(PoVError::Malformed);
		}
		let block_count = read_compact_len(&mut input)?;
		let mut last_header = None;
		for _ in 0..block_count {
			last_header = Some(skip_block(&mut input)?);
		}
		// The proof's encoded-nodes vec follows; its contents are irrelevant.
		last_header.ok_or(PoVError::Malformed)
	} else {
		// Legacy V0: a single bare `Block`, no version prefix. A zstd prefix
		// was already rejected above, so a V0 PoV must parse as a header.
		skip_block(&mut pov_to_input(pov))
	}
}

type Reader<'a> = &'a [u8];

fn pov_to_input(pov: &[u8]) -> Reader<'_> {
	pov
}

/// A `u32` `Compact` length (the `Vec` prefix).
fn read_compact_len(input: &mut Reader) -> Result<u64, PoVError> {
	let len = Compact::<u32>::decode(input).map_err(|_| PoVError::Malformed)?;
	Ok(u64::from(u32::from(len)))
}

fn read_byte(input: &mut Reader) -> Result<u8, PoVError> {
	let (&b, rest) = input.split_first().ok_or(PoVError::Malformed)?;
	*input = rest;
	Ok(b)
}

/// Skip a substrate `Digest` (`Vec<DigestItem>`). Each item is a length-prefixed
/// payload preceded by a single-byte variant tag.
fn skip_digest(input: &mut Reader) -> Result<(), PoVError> {
	let count = read_compact_len(input)?;
	for _ in 0..count {
		let tag = read_byte(input)?;
		match tag {
			// Other(Vec<u8>)
			0 => skip_vec_u8(input)?,
			// Consensus / Seal / PreRuntime: ([u8;4], Vec<u8>)
			1 | 2 | 3 => {
				if input.len() < 4 {
					return Err(PoVError::Malformed);
				}
				*input = &input[4..];
				skip_vec_u8(input)?;
			},
			// RuntimeEnvironmentUpdated: unit
			4 => {},
			_ => return Err(PoVError::Malformed),
		}
	}
	Ok(())
}

/// Skip a `Vec<u8>` (compact length + bytes).
fn skip_vec_u8(input: &mut Reader) -> Result<(), PoVError> {
	let len = read_compact_len(input)? as usize;
	if input.len() < len {
		return Err(PoVError::Malformed);
	}
	*input = &input[len..];
	Ok(())
}

/// Skip the `Vec<OpaqueExtrinsic>` block body (each extrinsic is its own
/// compact-length-prefixed blob).
fn skip_extrinsics(input: &mut Reader) -> Result<(), PoVError> {
	let count = read_compact_len(input)?;
	for _ in 0..count {
		skip_vec_u8(input)?;
	}
	Ok(())
}

/// Walk one substrate `Block`, returning the byte slice of its encoded header —
/// the para head — and leaving `input` past the block's body.
fn skip_block(input: &mut Reader) -> Result<Vec<u8>, PoVError> {
	let start = *input;
	skip_header(input)?;
	let header_len = start.len() - input.len();
	skip_extrinsics(input)?;

	let header = &start[..header_len];
	if header.len() as u32 > parachain_service_interface::types::MAX_HEAD_DATA_SIZE {
		return Err(PoVError::Malformed);
	}
	Ok(header.to_vec())
}

/// Skip a substrate `Header<u32, BlakeTwo256>`.
fn skip_header(input: &mut Reader) -> Result<(), PoVError> {
	// parent_hash (32) + compact number + state_root (32) + extrinsics_root (32)
	if input.len() < 32 {
		return Err(PoVError::Malformed);
	}
	*input = &input[32..];
	read_compact_len(input)?;
	if input.len() < 64 {
		return Err(PoVError::Malformed);
	}
	*input = &input[64..];
	skip_digest(input)
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::Encode;
	use parachain_service_interface::types::HeadData;

	/// Encode `(number, digest)` as a substrate `Header<u32>`.
	fn header_bytes(number: u32, digest: &[u8]) -> Vec<u8> {
		let mut h = Vec::new();
		h.extend_from_slice(&[0u8; 32]); // parent_hash
		h.extend_from_slice(&Compact::from(number).encode());
		h.extend_from_slice(&[0u8; 32]); // state_root
		h.extend_from_slice(&[0u8; 32]); // extrinsics_root
		h.extend_from_slice(digest);
		h
	}

	/// Encode a substrate `Block`: header + `Vec<OpaqueExtrinsic>`.
	fn block_with(header: Vec<u8>, extrinsics: Vec<Vec<u8>>) -> Vec<u8> {
		let mut b = header;
		Compact::from(extrinsics.len() as u32).encode_to(&mut b);
		for e in extrinsics {
			Compact::from(e.len() as u32).encode_to(&mut b);
			b.extend_from_slice(&e);
		}
		b
	}

	fn v1_pov(blocks: Vec<Vec<u8>>, proof_nodes: usize) -> Vec<u8> {
		let mut pov = VERSIONED_PREFIX.to_vec();
		pov.push(V1_VERSION);
		Compact::from(blocks.len() as u32).encode_to(&mut pov);
		for b in &blocks {
			pov.extend_from_slice(b);
		}
		Compact::from(proof_nodes as u32).encode_to(&mut pov);
		for _ in 0..proof_nodes {
			Compact::from(0u32).encode_to(&mut pov);
		}
		pov
	}

	const EMPTY_DIGEST: &[u8] = &[0x00];

	#[test]
	fn v1_last_header_is_head_works() {
		let first = block_with(header_bytes(1, EMPTY_DIGEST), vec![vec![0xAA; 8]]);
		let last = block_with(header_bytes(2, EMPTY_DIGEST), vec![]);
		let pov = v1_pov(vec![first, last.clone()], 0);

		let head = decode_para_head(&pov).expect("valid V1 parses");
		let header_only: HeadData = last[..last.len() - 1].to_vec().try_into().expect("fits");
		assert_eq!(head.as_slice(), &header_only[..]);
		assert_eq!(head.len(), 32 + 1 + 32 + 32 + 1);
	}

	#[test]
	fn v1_digest_items_parsed_works() {
		// A header with one PreRuntime and one Seal item, as a real collator
		// header carries; the walker must consume single-byte tags.
		let mut digest = Compact::from(2u32).encode();
		digest.push(3); // PreRuntime
		digest.extend_from_slice(&[0u8; 4]); // ConsensusEngineId
		vec![1u8, 2u8, 3u8].encode_to(&mut digest); // data
		digest.push(2); // Seal
		digest.extend_from_slice(&[0u8; 4]); // ConsensusEngineId
		vec![0xABu8; 32].encode_to(&mut digest); // signature

		let block = block_with(header_bytes(7, &digest), vec![]);
		let pov = v1_pov(vec![block.clone()], 0);

		let head = decode_para_head(&pov).expect("digest walker parses real tags");
		let header_only: HeadData = block[..block.len() - 1].to_vec().try_into().expect("fits");
		assert_eq!(head.as_slice(), &header_only[..]);
	}

	#[test]
	fn legacy_v0_works() {
		let block = block_with(header_bytes(3, EMPTY_DIGEST), vec![]);
		let mut v0 = block.clone();
		Compact::from(0u32).encode_to(&mut v0); // CompactProof: empty encoded_nodes
		let head = decode_para_head(&v0).expect("V0 parses");
		let header_only: HeadData = block[..block.len() - 1].to_vec().try_into().expect("fits");
		assert_eq!(head.as_slice(), &header_only[..]);
	}

	#[test]
	fn compressed_rejected_works() {
		let mut pov = ZSTD_PREFIX.to_vec();
		pov.extend_from_slice(&[0u8; 16]);
		assert_eq!(decode_para_head(&pov), Err(PoVError::Compressed));
	}

	#[test]
	fn malformed_rejected_works() {
		assert_eq!(decode_para_head(&[]), Err(PoVError::Malformed));
		assert_eq!(decode_para_head(b"VERSIONEDPBD"), Err(PoVError::Malformed));
	}
}