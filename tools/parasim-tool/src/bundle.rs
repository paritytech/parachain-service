//! Assembling a work-package bundle whose import data travels inline.
//!
//! Guarantors normally fetch a package's import segments from JAM's data availability. That cannot
//! work for a pipelined chain: the child is submitted moments after its parent, long before the
//! parent's bundle has been made available, so the parent's segment 0 does not exist in DA yet.
//! The only party that can supply it is the submitter, which built the parent block in the first
//! place — so the bundle carries the segment and its Merkle proof. This is not an optimisation; it
//! is the only way the child can be guaranteed at all.
//!
//! No trust is added by it. The guarantor still verifies each inline segment's proof against a
//! segment root, and for an `Indirect` root the chain then validates that root against the parent
//! package's own report.
//!
//! Since phase 5a parasim ignores imports entirely — accumulate's reorder buffer carries the
//! lineage — so what is left here only keeps the current package format guaranteeable. It goes
//! away with the collator's `export_count = 0`.

use codec::Encode as _;
use jam_std_common::{build_encoded_bundle, import_proofs, ImportData};
use jam_types::{SegmentBytes, SegmentTreeRoot, WorkPackage, WorkPackageHash, SEGMENT_LEN};

/// parasim's export convention: segment 0 is the SCALE length-prefixed head, zero-padded.
///
/// The length prefix is parity-scale-codec, not jam-codec: this is the service's own byte
/// contract, and the service reads it back with `codec::Compact`.
pub fn head_segment(header: &[u8]) -> Vec<u8> {
	let mut segment = header.encode();
	segment.resize(SEGMENT_LEN, 0);
	segment
}

/// What JAM exports in place of a work item whose refine failed: zero-segments, as many as the
/// item declared. A child of a failed package imports exactly this.
pub fn zero_segment() -> Vec<u8> {
	vec![0u8; SEGMENT_LEN]
}

/// The inline import data for a single segment, plus the segment-tree root it commits to.
pub fn import_data(segment: Vec<u8>) -> (ImportData, SegmentTreeRoot) {
	let bytes = SegmentBytes::try_from(segment.clone())
		.expect("callers build segments of exactly SEGMENT_LEN; qed");
	let (mut proofs, root) = import_proofs(&[bytes]);
	(ImportData { segment, proof: proofs.remove(0) }, root)
}

/// Encode the bundle, returning the work-package hash alongside it.
///
/// The hash is over the *jam-codec* encoding of the package, which is what `build_encoded_bundle`
/// puts at the front of the bundle — so taking it from here cannot drift from what was submitted.
pub fn build(package: &WorkPackage, imports: Vec<ImportData>) -> (WorkPackageHash, Vec<u8>) {
	build_encoded_bundle(package, Vec::<Vec<u8>>::new(), &[imports])
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::Decode as _;

	/// The segment still has to be a well-formed one — a full-length segment holding the
	/// length-prefixed header — or the guarantor rejects the bundle before parasim ever runs.
	#[test]
	fn head_segment_round_trips_works() {
		let mut header = [7u8; 32].to_vec();
		codec::Compact(9u32).encode_to(&mut header);
		header.extend_from_slice(&[0u8; 64]); // state_root + extrinsics_root
		header.push(0); // empty Digest

		let segment = head_segment(&header);
		assert_eq!(segment.len(), SEGMENT_LEN);
		assert_eq!(Vec::<u8>::decode(&mut &segment[..]), Ok(header));
	}
}
