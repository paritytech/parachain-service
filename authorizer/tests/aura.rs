use codec::{DecodeAll, Encode, MaxEncodedLen};
use parachain_authorizer::aura::AuthTrace;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;

#[test]
fn auth_trace_mel_is_sane() {
	// Per GP an `AuthTrace` has to fit within `W_R`; we even hold it to a tighter 32 bytes.
	assert!(AuthTrace::max_encoded_len() <= 32);
	assert!(32 <= MAX_REFINE_OUTPUT_SIZE);
}

/// The trace is the one thing the authorizer blob and the service blob agree on without sharing
/// a build: they are deployed independently, and the only thing joining them is these bytes. So
/// the shape is pinned as bytes, not as a type — a field appearing here is a live-network
/// incident, and it has happened: a 33rd byte (`author_key ++ sudo`) made every package's refine
/// trap on `decode_all` before the trace was cut back to the spec's `{ author_key }`.
#[test]
fn auth_trace_wire_shape_works() {
	let author_key = [0xab; 32];
	let encoded = AuthTrace { author_key }.encode();
	assert_eq!(encoded, author_key, "the trace is the author key and nothing else");

	let with_trailing_byte = [encoded.as_slice(), &[0x00]].concat();
	assert!(
		AuthTrace::decode_all(&mut &with_trailing_byte[..]).is_err(),
		"a wider trace must be rejected here rather than in the service's refine"
	);
}
