//! Parameter input for the JAM service entry points.
//!
//! Accepts either raw SCALE bytes (an already-encoded `RefineParams` /
//! `AccumulateParams`, fed to the guest verbatim) or a small, debugging-friendly
//! JSON form that we encode with `jam-codec`. Without `--input`, zero defaults are
//! used so the entry point can be invoked with no setup.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use jam_codec::Encode;
use jam_types::{AccumulateParams, RefineParams, WorkPackageHash, WorkPayload};
use serde::Deserialize;

use crate::service::Entry;

/// How to interpret an `--input` file.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// A JSON object (see [`RefineInput`] / [`AccumulateInput`]).
    Json,
    /// Raw, already-SCALE-encoded params bytes.
    Scale,
}

/// JSON form of `RefineParams`. `payload` / `package_hash` are strings: `0x…` hex,
/// otherwise literal UTF-8 bytes (`package_hash` must resolve to exactly 32 bytes).
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RefineInput {
    core_index: u16,
    item_index: u32,
    service_id: u32,
    payload: String,
    package_hash: String,
}

/// JSON form of `AccumulateParams`.
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
    let fmt = format.unwrap_or_else(|| infer_format(path, &bytes));

    match fmt {
        // Already-encoded params: pass through verbatim (the guest decodes them).
        Format::Scale => Ok(bytes),
        Format::Json => match entry {
            Entry::Refine => {
                let r: RefineInput =
                    serde_json::from_slice(&bytes).context("parsing refine params JSON")?;
                Ok(RefineParams {
                    core_index: r.core_index,
                    item_index: r.item_index,
                    service_id: r.service_id,
                    payload: WorkPayload(parse_bytes(&r.payload)?),
                    package_hash: WorkPackageHash(parse_hash(&r.package_hash)?),
                }
                .encode())
            }
            Entry::Accumulate => {
                let a: AccumulateInput =
                    serde_json::from_slice(&bytes).context("parsing accumulate params JSON")?;
                Ok(AccumulateParams {
                    slot: a.slot,
                    service_id: a.service_id,
                    item_count: a.item_count,
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
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => Format::Json,
        Some("bin") | Some("scale") => Format::Scale,
        // Otherwise sniff: JSON objects start with `{` after optional whitespace.
        _ => match bytes.iter().find(|b| !b.is_ascii_whitespace()) {
            Some(b'{') => Format::Json,
            _ => Format::Scale,
        },
    }
}

/// `0x…` hex, otherwise the literal UTF-8 bytes of the string.
fn parse_bytes(s: &str) -> Result<Vec<u8>> {
    match s.strip_prefix("0x") {
        Some(hex) => hex::decode(hex).map_err(|e| anyhow!("invalid hex `{s}`: {e}")),
        None => Ok(s.as_bytes().to_vec()),
    }
}

fn parse_hash(s: &str) -> Result<[u8; 32]> {
    if s.is_empty() {
        return Ok([0u8; 32]);
    }
    let bytes = parse_bytes(s)?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| anyhow!("expected a 32-byte hash, got {} bytes", v.len()))
}
