//! Execute parachain service entry points with polkajam's in-memory node host.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use jam_node::{
    vm::{
        AccumulateCallContext, Engine, RefineCallContext, RefineCallContextOwned, StateMutations,
        Storage,
    },
    PvmBackend,
};
use jam_std_common::{hash_raw, Entropy, Privileges, Service};
use jam_types::{
    AuthConfig, Authorization, Authorizer, CodeHash, CoreIndex, ExtrinsicHash, ExtrinsicSpec,
    FixedVec, RefineContext,
};

pub use jam_types::{
    AccumulateItem, AuthTrace, Hash, Segment, ServiceId, WorkItem, WorkItemRecord, WorkOutput,
    WorkPackage, WorkPayload,
};

/// Service id used by the lightweight executor contexts.
pub const SERVICE_ID: ServiceId = 0;

/// Gas budget used by the lightweight executor contexts.
pub const GAS: u64 = 5_000_000_000;

/// Result of a refine invocation.
#[derive(Debug)]
pub struct RefineOutcome {
    pub output: WorkOutput,
    pub elapsed: Duration,
    pub gas_used: u64,
    pub exports: Vec<Segment>,
}

/// Result of an accumulate invocation.
#[derive(Debug)]
pub struct AccumulateOutcome {
    pub yielded: Option<Hash>,
    pub elapsed: Duration,
    pub gas_used: u64,
}

/// Result of an is_authorized invocation.
#[derive(Debug)]
pub struct AuthorizeOutcome {
    pub auth_trace: AuthTrace,
    pub elapsed: Duration,
    pub gas_used: u64,
}

/// Hash a PVM blob using the hash expected by JAM code storage.
pub fn blob_hash(blob: &[u8]) -> Hash {
    hash_raw(blob)
}

/// A work item paired with the raw bytes of its extrinsics.
///
/// [`WorkItem::extrinsics`] only records each extrinsic's hash and length; the
/// bytes themselves are supplied out-of-band through the refine context's
/// `extrinsic_data`. Bundling them keeps a single source of truth so the
/// per-item extrinsic count and the bytes fetched by `refine` cannot drift apart.
pub struct WorkItemWithExtrinsics {
    pub item: WorkItem,
    pub extrinsics: Vec<Vec<u8>>,
}

/// Construct a minimal work item that invokes `service_blob`.
///
/// `extrinsics` holds one `Vec<u8>` per extrinsic; each becomes an
/// [`ExtrinsicSpec`] (hash + length) on the [`WorkItem`], while the bytes are
/// carried alongside for the refine context to expose via `refine::extrinsic`.
pub fn work_item(
    service_blob: &[u8],
    payload: Vec<u8>,
    extrinsics: Vec<Vec<u8>>,
) -> WorkItemWithExtrinsics {
    let specs: Vec<ExtrinsicSpec> = extrinsics
        .iter()
        .map(|bytes| ExtrinsicSpec {
            hash: ExtrinsicHash(hash_raw(bytes)),
            len: bytes.len() as u32,
        })
        .collect();
    let item = WorkItem {
        service: SERVICE_ID,
        code_hash: CodeHash(blob_hash(service_blob)),
        refine_gas_limit: GAS,
        accumulate_gas_limit: 0,
        export_count: 0,
        payload: WorkPayload(payload),
        import_segments: Default::default(),
        extrinsics: specs
            .try_into()
            .expect("extrinsic count exceeds the JAM bound"),
    };
    WorkItemWithExtrinsics { item, extrinsics }
}

/// Execute a service blob's refine entry point with a minimal node call context.
pub fn refine(
    service_blob: &[u8],
    authorizer_blob: &[u8],
    work_items: Vec<WorkItemWithExtrinsics>,
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

    // Split the bundles: the specs go into the work package, the raw bytes into
    // the context's `extrinsic_data` (indexed `[work_item][extrinsic]`, which is
    // what backs the `refine::extrinsic(index)` host call).
    let mut items = Vec::with_capacity(work_items.len());
    let mut extrinsic_data = Vec::with_capacity(work_items.len());
    for bundle in work_items {
        items.push(bundle.item);
        extrinsic_data.push(bundle.extrinsics);
    }

    let work_package = work_package(
        items,
        blob_hash(authorizer_blob),
        Default::default(),
        Default::default(),
    )?;

    let mut context = RefineCallContextOwned {
        storage,
        core: 0,
        service_id: SERVICE_ID,
        gas: GAS,
        auth_output: Default::default(),
        import_data: vec![],
        extrinsic_data: extrinsic_data
            .into_iter()
            .map(|per_item| per_item.into_iter().map(Into::into).collect())
            .collect(),
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

/// Execute an authorizer blob's is_authorized entry point with a minimal node call context.
///
/// The authorizer is a separate program from the service, with its own blob and
/// code hash. `config` becomes the authorizer's `config` blob (pinned by the
/// Coretime chain; it begins with the `Vec<ParaId>` the service is authorized
/// for) and `token` becomes the work package's `authorization` (the collator's
/// per-package token). The returned [`AuthTrace`] is what refine/accumulate
/// later observe.
pub fn is_authorized(
    authorizer_blob: &[u8],
    config: Vec<u8>,
    token: Vec<u8>,
    core: CoreIndex,
) -> Result<AuthorizeOutcome> {
    let (storage, authorizer_code_hash) = storage_with_code(authorizer_blob)?;
    let package = work_package(
        Vec::new(),
        authorizer_code_hash,
        Authorization(token),
        AuthConfig(config),
    )?;

    let engine = interpreter_engine()?;
    let start = Instant::now();
    let (result, gas_used) = engine.is_authorized(&package.into(), core, &storage, None);
    let elapsed = start.elapsed();
    let auth_trace = result.map_err(|error| anyhow!("is_authorized failed: {error}"))?;

    Ok(AuthorizeOutcome {
        auth_trace,
        elapsed,
        gas_used,
    })
}

fn work_package(
    work_items: Vec<WorkItem>,
    authorizer_code_hash: Hash,
    authorization: Authorization,
    config: AuthConfig,
) -> Result<WorkPackage> {
    Ok(WorkPackage {
        authorization,
        auth_code_host: SERVICE_ID,
        authorizer: Authorizer {
            code_hash: CodeHash(authorizer_code_hash),
            config,
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
