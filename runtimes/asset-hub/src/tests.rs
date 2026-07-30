use std::path::{Path, PathBuf};

use executor::jam;
use jam_codec::Decode;
use jam_types::{AuthTrace, WorkPayload};

#[test]
fn refine_runs_for_asset_hub() {
    let service = blob("parachain-service.jam");
    let authorizer = blob("parachain-authorizer.jam");
    let runtime = read(
        repo_target()
            .join("release")
            .join("rbuild")
            .join("asset-hub")
            .join("asset-hub-blob.polkavm"),
    );

    // Until the service invokes `jam_validate_block`, carry the runtime identity
    // as the work payload. This keeps the runtime-owned refine test connected to
    // the exact Asset Hub artifact it is intended to validate.
    let payload = jam::blob_hash(&runtime).to_vec();
    let work_items = vec![jam::work_item(&service, payload.clone())];
    let outcome = jam::refine(&service, &authorizer, work_items, 0)
        .expect("Asset Hub refine should run to completion");

    assert!(outcome.gas_used > 0);
    let (_, refined_payload, _) =
        <(u64, WorkPayload, AuthTrace)>::decode(&mut outcome.output.as_slice())
            .expect("service refine output should decode");
    assert_eq!(refined_payload.0, payload);
}

fn blob(name: &str) -> Vec<u8> {
    read(repo_target().join(name))
}

fn repo_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
}

fn read(path: PathBuf) -> Vec<u8> {
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "reading {}: {error}\nBuild it first with `just build`.",
            path.display()
        )
    })
}
