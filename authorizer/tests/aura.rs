use codec::MaxEncodedLen;
use parachain_authorizer::aura::AuthTrace;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;

#[test]
fn auth_trace_mel_is_sane() {
	// Per GP an `AuthTrace` has to fit within `W_R`; we even hold it to a tighter 32 bytes.
	assert!(AuthTrace::max_encoded_len() <= 32);
	assert!(32 <= MAX_REFINE_OUTPUT_SIZE);
}
