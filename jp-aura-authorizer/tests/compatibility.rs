//! Fixed protocol vectors for SCALE encoding and the zero-padded Merkle tree.
use codec::Encode;
use jp_aura_authorizer::{build_collator_tree, AuthConfig, AuthToken, AuthTrace, ParaId};
use primitive_types::H256;

#[test]
fn padded_tree_vector_works() {
	let (root, proofs) = build_collator_tree(&[[1; 32], [2; 32], [3; 32]]);
	assert_eq!(
		root,
		H256::from([
			85, 243, 163, 207, 64, 253, 63, 18, 253, 224, 77, 156, 237, 59, 240, 105, 126, 129,
			244, 235, 59, 193, 124, 86, 112, 220, 83, 111, 50, 97, 27, 205
		])
	);
	assert_eq!(
		proofs[2],
		vec![
			H256::zero(),
			H256::from([
				180, 31, 7, 124, 161, 36, 245, 239, 83, 83, 112, 218, 244, 28, 116, 162, 188, 72,
				186, 126, 184, 104, 231, 51, 124, 193, 247, 244, 171, 255, 28, 174
			])
		]
	);
}

#[test]
fn scale_wire_vectors_works() {
	let config = AuthConfig {
		para_ids: vec![ParaId::new(1000)],
		parachain_service: 1337,
		collator_set_root: H256::zero(),
		collator_set_size: 3,
		slot_duration: 2,
	};
	let mut expected = vec![4, 232, 3, 0, 0, 57, 5, 0, 0];
	expected.extend_from_slice(&[0; 32]);
	expected.extend_from_slice(&[3, 0, 0, 0, 2, 0, 0, 0]);
	assert_eq!(config.encode(), expected);
	let token = AuthToken { proof: vec![H256::repeat_byte(7)], key: [8; 32], signature: [9; 64] };
	let mut expected = vec![4];
	expected.extend_from_slice(&[7; 32]);
	expected.extend_from_slice(&[8; 32]);
	expected.extend_from_slice(&[9; 64]);
	assert_eq!(token.encode(), expected);
	for sudo in [false, true] {
		let mut expected = vec![8; 32];
		expected.push(u8::from(sudo));
		assert_eq!(AuthTrace { author_key: [8; 32], sudo }.encode(), expected);
	}
}
