use codec::{DecodeAll, Encode, MaxEncodedLen};
use parachain_authorizer::aura::AuthTrace;
use parachain_service::work_digest::MAX_REFINE_OUTPUT_SIZE;
use parachain_service_core::authorization::DevelopmentAuthTrace;

#[test]
fn auth_trace_size_works() {
	// Keep the fixed-size trace within the service output budget.
	assert!(DevelopmentAuthTrace::max_encoded_len() <= 96);
	assert!(96 <= MAX_REFINE_OUTPUT_SIZE);
}

#[test]
fn development_trace_wire_works() {
	for sudo in [false, true] {
		let trace = DevelopmentAuthTrace { aura: AuthTrace { author_key: [8; 32] }, sudo };
		let mut expected = vec![8; 32];
		expected.push(u8::from(sudo));
		assert_eq!(trace.encode(), expected);
		assert_eq!(DevelopmentAuthTrace::decode_all(&mut &expected[..]).unwrap(), trace);
	}
}
