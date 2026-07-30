//! Execute parachain service entry points with polkajam's in-memory node host.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use jam_node::{
    vm::{
        AccumulateCallContext, Engine, RefineCallContext, RefineCallContextOwned, StateMutations,
        Storage,
    },
    PvmBackend,
};
use jam_std_common::{hash_raw, Entropy, Privileges, Service};
use jam_types::{Authorizer, CodeHash, FixedVec, RefineContext};

pub use jam_types::{
    AccumulateItem, Hash, Segment, ServiceId, WorkItem, WorkItemRecord, WorkOutput, WorkPackage,
    WorkPayload,
};

/// Service id used by the lightweight executor contexts.
pub const SERVICE_ID: ServiceId = 0;

/// Gas budget used by the lightweight executor contexts.
pub const GAS: u64 = 5_000_000_000;

/// Result of a refine invocation.
pub struct RefineOutcome {
    pub output: WorkOutput,
    pub elapsed: Duration,
    pub gas_used: u64,
    pub exports: Vec<Segment>,
}

/// Result of an accumulate invocation.
pub struct AccumulateOutcome {
    pub yielded: Option<Hash>,
    pub elapsed: Duration,
    pub gas_used: u64,
}

/// Hash a PVM blob using the hash expected by JAM code storage.
pub fn blob_hash(blob: &[u8]) -> Hash {
    hash_raw(blob)
}

/// Construct a minimal work item that invokes `service_blob`.
pub fn work_item(service_blob: &[u8], payload: Vec<u8>) -> WorkItem {
    WorkItem {
        service: SERVICE_ID,
        code_hash: CodeHash(blob_hash(service_blob)),
        refine_gas_limit: GAS,
        accumulate_gas_limit: 0,
        export_count: 0,
        payload: WorkPayload(payload),
        import_segments: Default::default(),
        extrinsics: Default::default(),
    }
}

/// Execute a service blob's refine entry point with a minimal node call context.
pub fn refine(
    service_blob: &[u8],
    authorizer_blob: &[u8],
    work_items: Vec<WorkItem>,
    work_item_index: usize,
) -> Result<RefineOutcome> {
    if work_items.is_empty() {
        bail!("refine requires at least one work item");
    }
    if work_item_index >= work_items.len() {
        bail!(
            "work item index {work_item_index} is out of bounds for {} items",
            work_items.len()
        );
    }

    let (storage, code_hash) = storage_with_code(service_blob)?;
    let work_package = work_package(work_items, blob_hash(authorizer_blob))?;

    let mut context = RefineCallContextOwned {
        storage,
        core: 0,
        service_id: SERVICE_ID,
        gas: GAS,
        auth_output: Default::default(),
        import_data: vec![],
        extrinsic_data: vec![],
        export_counter: 0,
        max_exports: 1024,
        exports: vec![],
        work_package: work_package.into(),
        work_item_index,
        output_len: 0,
        engine: interpreter_engine()?,
    };

    let engine = interpreter_engine()?;
    let (result, elapsed, gas_used) =
        engine.refine(CodeHash(code_hash), RefineCallContext::from(&mut context));
    let output = result.map_err(|error| anyhow!("refine failed: {error}"))?;

    Ok(RefineOutcome {
        output,
        elapsed,
        gas_used,
        exports: context.exports,
    })
}

/// Execute a service blob's accumulate entry point with a minimal node call context.
pub fn accumulate(service_blob: &[u8], items: Vec<AccumulateItem>) -> Result<AccumulateOutcome> {
    let (storage, code_hash) = storage_with_code(service_blob)?;
    let mut context = AccumulateCallContext {
        storage,
        mutations: StateMutations::new(0),
        snapshot: None,
        gas: GAS,
        slot: 0,
        service_id: SERVICE_ID,
        entropy: Entropy::default(),
        items,
        privileges: Privileges {
            bless: SERVICE_ID,
            assign: FixedVec::new(SERVICE_ID),
            designate: SERVICE_ID,
            register: SERVICE_ID,
            always_acc: Default::default(),
        },
        cost: None,
    };

    let engine = interpreter_engine()?;
    let (result, elapsed, gas_used) = engine.accumulate(CodeHash(code_hash), &mut context);
    let yielded = result.map_err(|error| anyhow!("accumulate failed: {error}"))?;

    Ok(AccumulateOutcome {
        yielded,
        elapsed,
        gas_used,
    })
}

fn work_package(work_items: Vec<WorkItem>, authorizer_code_hash: Hash) -> Result<WorkPackage> {
    Ok(WorkPackage {
        authorization: Default::default(),
        auth_code_host: SERVICE_ID,
        authorizer: Authorizer {
            code_hash: CodeHash(authorizer_code_hash),
            config: Default::default(),
        },
        context: RefineContext {
            anchor: Default::default(),
            state_root: Default::default(),
            beefy_root: Default::default(),
            lookup_anchor: Default::default(),
            lookup_anchor_slot: 0,
            prerequisites: Default::default(),
        },
        items: work_items
            .try_into()
            .map_err(|_| anyhow!("work-item count exceeds the JAM bound"))?,
    })
}

fn storage_with_code(blob: &[u8]) -> Result<(Storage, Hash)> {
    let code_hash = blob_hash(blob);
    let service = Service {
        code_hash: CodeHash(code_hash),
        balance: 1_000_000_000_000,
        min_item_gas: 100,
        min_memo_gas: 100,
        bytes: 1_000_000,
        items: 1_000,
        deposit_offset: 0,
        creation_slot: 0,
        last_accumulation_slot: 0,
        parent_service: 0,
    };
    let mut storage = Storage::new_empty();
    storage.set_service(SERVICE_ID, &service);
    storage
        .solicit(0, SERVICE_ID, code_hash, blob.len() as u32)
        .map_err(|error| anyhow!("soliciting service code failed: {error:?}"))?;
    storage
        .provide(0, SERVICE_ID, blob)
        .map_err(|error| anyhow!("providing service code failed: {error:?}"))?;
    storage.commit();
    Ok((storage, code_hash))
}

fn interpreter_engine() -> Result<Engine> {
    Engine::new(Some(PvmBackend::Interpreter))
        .map_err(|error| anyhow!("creating interpreter engine failed: {error}"))
}
