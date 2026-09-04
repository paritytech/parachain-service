use codec::{DecodeAll, Encode, MaxEncodedLen};
use parachain_authorizer::aura::AuthTrace;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;

#[test]
fn auth_trace_mel_is_sane() {
	// Per GP an `AuthTrace` has to fit within `W_R`; we even hold it to a tighter 33 bytes.
	assert!(AuthTrace::max_encoded_len() <= 33);
	assert!(33 <= MAX_REFINE_OUTPUT_SIZE);
}

#[test]
fn auth_trace_deployed_wire_shape_works() {
	// The deployed collator stack's sr25519 authorizer emits
	// `author_key ++ sudo` (33 bytes); refine must speak that wire shape.
	let mut wire = vec![7u8; 32];
	wire.push(0x00);

	let trace = AuthTrace::decode_all(&mut &wire[..]).expect("deployed shape must decode");

	assert_eq!(trace.encode(), wire);
}
