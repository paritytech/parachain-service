//! Service entry-point tests running against polkajam's in-memory node host.
//!
//! One module per entry point: [`refine`], [`authorize`] and [`accumulate`].

use std::path::PathBuf;

mod accumulate;
mod authorize;
mod refine;

fn service_blob() -> Vec<u8> {
    blob("parachain-service.jam")
}

fn authorizer_blob() -> Vec<u8> {
    blob("parachain-authorizer.jam")
}

fn blob(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "reading {}: {error}\nBuild it first with `just build`.",
            path.display()
        )
    })
}
