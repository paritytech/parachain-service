use codec::MaxEncodedLen;
use parachain_authorizer::aura::AuthTrace;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;

#[test]
fn auth_trace_size_works() {
	// Keep the fixed-size trace within the service output budget.
	assert!(AuthTrace::max_encoded_len() <= 96);
	assert!(96 <= MAX_REFINE_OUTPUT_SIZE);
}
