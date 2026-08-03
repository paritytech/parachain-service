//! Thin adapters around PolkaJAM's in-memory executor.

use anyhow::{anyhow, Result};
use codec::DecodeAll;
use jam_node::vm::{
    AccumulateCallContext, Engine, RefineCallContext, RefineCallContextOwned, Storage,
};
use jam_types::{AuthTrace, CodeHash, CoreIndex, Hash, WorkOutput, WorkPackage};
use parachain_service::work_digest::ParachainWorkDigest;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RefineOutcome {
    pub digest: ParachainWorkDigest,
    pub elapsed: Duration,
    pub gas_used: u64,
}

#[derive(Debug)]
pub struct AccumulateOutcome {
    pub yielded: Option<Hash>,
    pub elapsed: Duration,
    pub gas_used: u64,
}

#[derive(Debug)]
pub struct IsAuthorizedOutcome {
    pub auth_trace: AuthTrace,
    pub elapsed: Duration,
    pub gas_used: u64,
}

/// Run refine with a caller-built engine and call context.
pub fn refine(
    engine: &Engine,
    code_hash: CodeHash,
    context: &mut RefineCallContextOwned,
) -> Result<RefineOutcome> {
    let (result, elapsed, gas_used) = engine.refine(code_hash, RefineCallContext::from(context));
    let output = result.map_err(|error| anyhow!("refine failed: {error}"))?;
    let digest = ParachainWorkDigest::decode_all(&mut &output[..])
        .map_err(|e| anyhow!("refine produced an undecodable digest: {e}"))?;

    Ok(RefineOutcome {
        digest,
        elapsed,
        gas_used,
    })
}

/// Run accumulate with a caller-built engine and call context.
pub fn accumulate(
    engine: &Engine,
    code_hash: CodeHash,
    context: &mut AccumulateCallContext<'_>,
) -> Result<AccumulateOutcome> {
    let (result, elapsed, gas_used) = engine.accumulate(code_hash, context);
    let yielded = result.map_err(|error| anyhow!("accumulate failed: {error}"))?;

    Ok(AccumulateOutcome {
        yielded,
        elapsed,
        gas_used,
    })
}

/// Run is-authorized against a caller-built package and storage snapshot.
pub fn is_authorized(
    engine: &Engine,
    package: &WorkPackage,
    core: CoreIndex,
    storage: &Storage,
) -> Result<IsAuthorizedOutcome> {
    let package = package.into();
    let start = Instant::now();
    let (result, gas_used) = engine.is_authorized(&package, core, storage, None);
    let elapsed = start.elapsed();
    let auth_trace = result.map_err(|error| anyhow!("is_authorized failed: {error}"))?;

    Ok(IsAuthorizedOutcome {
        auth_trace,
        elapsed,
        gas_used,
    })
}
