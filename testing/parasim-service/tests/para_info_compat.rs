//! Byte-compat drift protection: parasim keeps its own minimal `ParaInfoLite`
//! so the guest stays clear of the churning service crate, but the stored
//! value must stay byte-for-byte the real `ParaInfo` (a collator decodes it as
//! the real type). This test pins that equality at every build.
//!
//! Only compiled when the `test-utils` feature is on (it pulls in
//! `parachain-service`, which parasim exists to avoid depending on).

#![cfg(feature = "test-utils")]

use codec::Encode;
use parachain_service::state::para_info::ParaInfo;
use parachain_service_interface::types::HeadData;

#[test]
fn parasim_para_info_is_byte_compatible() {
	let head = HeadData::try_from(vec![1u8, 2, 3, 4]).expect("fits");

	let real = ParaInfo {
		head_data: head.clone(),
		// The defaults parasim's `ParaInfoLite` implies: no code, no upgrade,
		// zero balances, not deregistering.
		validation_code: None,
		pending_upgrade: None,
		total_state_balance: 0u32.into(),
		used_state_balance: 0u32.into(),
		is_deregistering: false,
	};
	let lite = parasim_service::ParaInfoLite::with_head(head);

	assert_eq!(lite.encode(), real.encode());
}