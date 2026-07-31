use codec::Encode;
use executor::jam;
use jam_codec::Decode;
use jam_types::{AuthTrace, WorkPayload};
use parachain_support::types::ParaId;

use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::BLOB as SERVICE;

fn auth_config() -> jam::AuthConfig {
    jam::AuthConfig(vec![ParaId(1)].encode())
}

fn auth_token() -> jam::AuthToken {
    jam::AuthToken::new()
}

#[test]
fn refine_runs_for_asset_hub() {
    let runtime = crate::WASM_BINARY.expect("Asset Hub runtime blob is built with `std`");

    let code_hash = jam::blob_hash(runtime).to_vec();
    let payload = code_hash.encode();

    let para_state_proof = b"para-state-proof".to_vec();
    let jam_state_proof = b"jam-state-proof".to_vec();
    let work_items = vec![jam::work_item(
        SERVICE,
        payload.clone(),
        vec![para_state_proof, jam_state_proof],
    )];

    // Run the authorizer first (spec: `Ψ_I` before `Ψ_R`) and thread its trace
    // into refine, passing the same config/token so the work package matches.
    let auth_trace = jam::is_authorized(AUTHORIZER, auth_config(), auth_token(), 0)
        .expect("is_authorized should run to completion")
        .auth_trace;

    let outcome = jam::refine(
        SERVICE,
        AUTHORIZER,
        auth_config(),
        auth_token(),
        auth_trace,
        work_items,
        0,
    )
    .expect("Asset Hub refine should run to completion");

    assert!(outcome.gas_used > 0);
}
