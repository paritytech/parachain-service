use codec::MaxEncodedLen;
use parachain_authorizer::aura::AuthTrace;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;

#[test]
fn auth_trace_mel_is_sane() {
	// Per GP: An AuthTrace has to fit within `W_R`.
	assert!(AuthTrace::max_encoded_len() <= MAX_REFINE_OUTPUT_SIZE);
}
