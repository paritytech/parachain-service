use executor::jam;
use jam_codec::Decode;
use jam_types::{AuthTrace, WorkPayload};
use codec::Encode;

use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::BLOB as SERVICE;

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
    let outcome = jam::refine(SERVICE, AUTHORIZER, work_items, 0)
        .expect("Asset Hub refine should run to completion");

    assert!(outcome.gas_used > 0);
    let (_, refined_payload, _) =
        <(u64, WorkPayload, AuthTrace)>::decode(&mut outcome.output.as_slice())
            .expect("service refine output should decode");
    assert_eq!(refined_payload.0, payload);
}
