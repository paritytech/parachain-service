use codec::MaxEncodedLen;
use parachain_authorizer::aura::AuthTrace;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;

#[test]
fn auth_trace_mel_is_sane() {
	// Per GP an `AuthTrace` has to fit within `W_R`; we hold it to a much tighter bound, since
	// everything in it is fixed-width: a 32-byte author key and an optional core-assignment
	// command. Anything unbounded getting in here would show up as this failing.
	assert!(AuthTrace::max_encoded_len() <= 96);
	assert!(96 <= MAX_REFINE_OUTPUT_SIZE);
}
