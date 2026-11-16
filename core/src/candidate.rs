//! Parachain candidate payload: shared between the collator and the service host.

use codec::{Decode, Encode};

use crate::types::{ParaId, ValidationCodeHash};

/// Parachain work-item payload (§3.2), SCALE-compatible with the SDK candidate.
/// The PoV is carried in extrinsic 0.
#[derive(Encode, Decode)]
pub struct ParachainCandidate {
	pub para_id: ParaId,
	pub validation_code: ValidationCodeHash,
}

#[cfg(test)]
mod tests {
	use super::*;
	use codec::DecodeAll;

	#[test]
	fn authorizer_wire_works() {
		let candidate =
			ParachainCandidate { para_id: ParaId::new(42), validation_code: [7; 32].into() };
		let encoded = candidate.encode();
		let decoded =
			cumulus_aura_authorizer_utils::ParachainCandidate::decode_all(&mut &encoded[..])
				.expect("authorizer must decode service candidates");
		assert_eq!(u32::from(decoded.para_id), 42);
		assert_eq!(decoded.validation_code.as_ref(), &[7; 32]);
		assert_eq!(decoded.encode(), encoded);
	}
}
