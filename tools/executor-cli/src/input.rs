//! Parameter input for the JAM service entry points.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use executor::service::Entry;
use jam_codec::Encode;
use jam_types::{AccumulateParams, RefineParams, WorkPackageHash, WorkPayload};
use serde::Deserialize;

/// How to interpret an `--input` file.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// A JSON object.
    Json,
    /// Raw, already-SCALE-encoded params bytes.
    Scale,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RefineInput {
    core_index: u16,
    item_index: u32,
    service_id: u32,
    payload: String,
    package_hash: String,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct AccumulateInput {
    slot: u32,
    service_id: u32,
    item_count: u32,
}

/// Produce the SCALE-encoded params bytes to hand to the guest.
pub fn load_params(entry: Entry, input: Option<&Path>, format: Option<Format>) -> Result<Vec<u8>> {
    let Some(path) = input else {
        return Ok(defaults(entry));
    };

    let bytes = std::fs::read(path).with_context(|| format!("reading input {}", path.display()))?;
    let format = format.unwrap_or_else(|| infer_format(path, &bytes));

    match format {
        Format::Scale => Ok(bytes),
        Format::Json => match entry {
            Entry::Refine => {
                let input: RefineInput =
                    serde_json::from_slice(&bytes).context("parsing refine params JSON")?;
                Ok(RefineParams {
                    core_index: input.core_index,
                    item_index: input.item_index,
                    service_id: input.service_id,
                    payload: WorkPayload(parse_bytes(&input.payload)?),
                    package_hash: WorkPackageHash(parse_hash(&input.package_hash)?),
                }
                .encode())
            }
            Entry::Accumulate => {
                let input: AccumulateInput =
                    serde_json::from_slice(&bytes).context("parsing accumulate params JSON")?;
                Ok(AccumulateParams {
                    slot: input.slot,
                    service_id: input.service_id,
                    item_count: input.item_count,
                }
                .encode())
            }
        },
    }
}

fn defaults(entry: Entry) -> Vec<u8> {
    match entry {
        Entry::Refine => RefineParams {
            core_index: 0,
            item_index: 0,
            service_id: 0,
            payload: WorkPayload(Vec::new()),
            package_hash: WorkPackageHash([0u8; 32]),
        }
        .encode(),
        Entry::Accumulate => AccumulateParams {
            slot: 0,
            service_id: 0,
            item_count: 0,
        }
        .encode(),
    }
}

fn infer_format(path: &Path, bytes: &[u8]) -> Format {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => Format::Json,
        Some("bin") | Some("scale") => Format::Scale,
        _ => match bytes.iter().find(|byte| !byte.is_ascii_whitespace()) {
            Some(b'{') => Format::Json,
            _ => Format::Scale,
        },
    }
}

fn parse_bytes(value: &str) -> Result<Vec<u8>> {
    match value.strip_prefix("0x") {
        Some(value) => {
            hex::decode(value).map_err(|error| anyhow!("invalid hex `0x{value}`: {error}"))
        }
        None => Ok(value.as_bytes().to_vec()),
    }
}

fn parse_hash(value: &str) -> Result<[u8; 32]> {
    if value.is_empty() {
        return Ok([0u8; 32]);
    }
    let bytes = parse_bytes(value)?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| anyhow!("expected a 32-byte hash, got {} bytes", bytes.len()))
}
