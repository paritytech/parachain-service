use executor::jam;
use jam_codec::Decode;
use jam_types::{AuthTrace, WorkPayload};

use parachain_authorizer_bin::BLOB as AUTHORIZER;
use parachain_service_bin::BLOB as SERVICE;

#[test]
fn refine_runs_for_asset_hub() {
    let runtime = crate::WASM_BINARY.expect("Asset Hub runtime blob is built with `std`");

    // Until the service invokes `jam_validate_block`, carry the runtime identity
    // as the work payload. This keeps the runtime-owned refine test connected to
    // the exact Asset Hub artifact it is intended to validate.
    let payload = jam::blob_hash(runtime).to_vec();
    let work_items = vec![jam::work_item(SERVICE, payload.clone())];
    let outcome = jam::refine(SERVICE, AUTHORIZER, work_items, 0)
        .expect("Asset Hub refine should run to completion");

    assert!(outcome.gas_used > 0);
    let (_, refined_payload, _) =
        <(u64, WorkPayload, AuthTrace)>::decode(&mut outcome.output.as_slice())
            .expect("service refine output should decode");
    assert_eq!(refined_payload.0, payload);
}
