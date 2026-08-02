//! Shared test fixtures for the blob integration tests (`tests/`).
//!
//! Gated behind the `test-utils` feature, which the crate turns on for its own
//! tests via a self dev-dependency (see `Cargo.toml`).

use codec::Encode;
use executor::pj::SERVICE_ID;
use jam_types::{AuthConfig, Authorization as AuthToken, CodeHash, WorkItem};
use parachain_authorizer::{AuraAuthConfig, AuthToken as CollatorAuthToken, ParaId};
use primitive_types::H256;

/// An authorizer config whose `ParaId` prefix authorizes `para_ids` packages.
pub fn good_config(para_ids: usize) -> AuthConfig {
    let para_ids = (0..para_ids).map(|i| ParaId(i as u32)).collect::<Vec<_>>();
    let config = AuraAuthConfig {
        para_ids,
        collator_set_root: H256::zero(),
        collator_set_size: 0,
        slot_duration: 0,
    };
    AuthConfig(config.encode())
}

/// An empty but well-formed Aura collator authorization token.
pub fn good_token() -> AuthToken {
    let token = CollatorAuthToken {
        proof: vec![H256::zero()],
        key: [0; 32],
        signature: [255; 64],
    };
    AuthToken(token.encode())
}

/// `n` minimal work items addressed to the parachain service.
pub fn work_items(n: usize) -> Vec<WorkItem> {
    (0..n)
        .map(|_| WorkItem {
            service: SERVICE_ID,
            code_hash: CodeHash::zero(),
            refine_gas_limit: 0,
            accumulate_gas_limit: 0,
            export_count: 0,
            payload: Default::default(),
            import_segments: Default::default(),
            extrinsics: Default::default(),
        })
        .collect()
}
