use codec::MaxEncodedLen;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;

use crate::aura::AuthTrace;

#[test]
fn auth_trace_mel_works() {
	// Per GP an `AuthTrace` has to fit within `W_R`.
	assert!(AuthTrace::max_encoded_len() <= MAX_REFINE_OUTPUT_SIZE);
}
