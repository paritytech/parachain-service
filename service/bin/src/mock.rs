//! Shared fixtures for the blob integration tests.

use codec::Encode;
use jam_node::{
    vm::{AccumulateCallContext, Engine, RefineCallContextOwned, StateMutations, Storage},
    PvmBackend,
};
use jam_std_common::{hash_raw, Entropy, Privileges, Service};
use jam_types::{
    AccumulateItem, AuthConfig, AuthTrace, Authorization as AuthToken, Authorizer, CodeHash,
    ExtrinsicHash, ExtrinsicSpec, FixedVec, RefineContext, ServiceId, WorkItem, WorkPackage,
    WorkPayload,
};
use parachain_authorizer::{aura, ParaId};
use primitive_types::H256;

const SERVICE_ID: ServiceId = 0;
const GAS_LIMIT: u64 = 5_000_000_000;

pub type RefineWorkItem = (WorkItem, Vec<Vec<u8>>);

/// An authorizer config whose `ParaId` prefix authorizes `para_ids` packages.
pub fn good_config(para_ids: usize) -> AuthConfig {
    let para_ids = (0..para_ids).map(|i| ParaId(i as u32)).collect::<Vec<_>>();
    let config = aura::AuthConfig {
        para_ids,
        collator_set_root: H256::zero(),
        collator_set_size: 0,
        slot_duration: 0,
    };

    AuthConfig(config.encode())
}

/// An empty but well-formed Aura collator authorization token.
pub fn good_token() -> AuthToken {
    let token = aura::AuthToken {
        proof: vec![H256::zero()],
        key: [0; 32],
        signature: [255; 64],
    };

    AuthToken(token.encode())
}

/// `n` minimal work items addressed to the parachain service.
pub fn work_items(n: usize) -> Vec<WorkItem> {
    (0..n)
        .map(|_| WorkItem {
            service: SERVICE_ID,
            code_hash: CodeHash::zero(),
            refine_gas_limit: 0,
            accumulate_gas_limit: 0,
            export_count: 0,
            payload: Default::default(),
            import_segments: Default::default(),
            extrinsics: Default::default(),
        })
        .collect()
}

pub fn refine_work_item(
    service_blob: &[u8],
    payload: Vec<u8>,
    extrinsics: Vec<Vec<u8>>,
) -> RefineWorkItem {
    let specs = extrinsics
        .iter()
        .map(|bytes| ExtrinsicSpec {
            hash: ExtrinsicHash(hash_raw(bytes)),
            len: bytes.len() as u32,
        })
        .collect::<Vec<_>>()
        .try_into()
        .expect("extrinsic count exceeds the JAM bound");

    let item = WorkItem {
        service: SERVICE_ID,
        code_hash: CodeHash(hash_raw(service_blob)),
        refine_gas_limit: GAS_LIMIT,
        accumulate_gas_limit: 0,
        export_count: 0,
        payload: WorkPayload(payload),
        import_segments: Default::default(),
        extrinsics: specs,
    };

    (item, extrinsics)
}

pub fn refine_args(
    service_blob: &[u8],
    authorizer_blob: &[u8],
    config: AuthConfig,
    token: AuthToken,
    auth_trace: AuthTrace,
    work_items: Vec<RefineWorkItem>,
    work_item_index: usize,
) -> (Engine, CodeHash, RefineCallContextOwned) {
    let (items, extrinsic_data): (Vec<_>, Vec<_>) = work_items.into_iter().unzip();
    let package = work_package(authorizer_blob, config, token, items);
    let (storage, code_hash) = storage_with_code(service_blob);

    let context = RefineCallContextOwned {
        storage,
        core: 0,
        service_id: SERVICE_ID,
        gas: GAS_LIMIT,
        auth_output: auth_trace,
        import_data: Vec::new(),
        extrinsic_data: extrinsic_data
            .into_iter()
            .map(|per_item| per_item.into_iter().map(Into::into).collect())
            .collect(),
        export_counter: 0,
        max_exports: 0,
        exports: Vec::new(),
        work_package: package.into(),
        work_item_index,
        output_len: 0,
        engine: engine(),
    };

    (engine(), code_hash, context)
}

pub fn is_authorized_args(
    authorizer_blob: &[u8],
    config: AuthConfig,
    token: AuthToken,
    work_items: Vec<WorkItem>,
) -> (Engine, WorkPackage, Storage) {
    let package = work_package(authorizer_blob, config, token, work_items);
    let (storage, _) = storage_with_code(authorizer_blob);
    (engine(), package, storage)
}

pub fn accumulate_args(
    service_blob: &[u8],
    items: Vec<AccumulateItem>,
) -> (Engine, CodeHash, AccumulateCallContext<'static>) {
    let (storage, code_hash) = storage_with_code(service_blob);
    let context = AccumulateCallContext {
        storage,
        mutations: StateMutations::new(0),
        snapshot: None,
        gas: GAS_LIMIT,
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

    (engine(), code_hash, context)
}

fn work_package(
    authorizer_blob: &[u8],
    config: AuthConfig,
    authorization: AuthToken,
    work_items: Vec<WorkItem>,
) -> WorkPackage {
    WorkPackage {
        authorization,
        auth_code_host: SERVICE_ID,
        authorizer: Authorizer {
            code_hash: CodeHash(hash_raw(authorizer_blob)),
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
            .expect("work-item count exceeds the JAM bound"),
    }
}

fn storage_with_code(blob: &[u8]) -> (Storage, CodeHash) {
    let code_hash = hash_raw(blob);
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
        .expect("service code should fit in storage");
    storage
        .provide(0, SERVICE_ID, blob)
        .expect("service code should be accepted by storage");
    storage.commit();
    (storage, CodeHash(code_hash))
}

fn engine() -> Engine {
    Engine::new(Some(PvmBackend::Interpreter)).expect("interpreter engine should initialize")
}
