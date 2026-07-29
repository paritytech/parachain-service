//! Tests that reuse the **polkajam node's** in-memory JAM host.
//!
//! Where [`crate::service`] drives our own PolkaVM 0.36 run-loop + a stub
//! [`crate::host`] (log + abort), these tests hand the built service blob to the
//! node's real `Engine` (PolkaVM 0.30) with its genuine `Storage` and full
//! host-call implementation, so `refine`/`accumulate` run against the same
//! semantics the live node uses.
//!
//! Test-only: the node crate (`jam-node`, a `[dev-dependency]`) uses an
//! undeclared unstable feature, so run these via `just service test`
//! (which sets `RUSTC_BOOTSTRAP=1`). Add `-- --include-ignored --nocapture`
//! and `RUST_LOG=jam_node=trace,pvm=trace` to watch the host-call trace.
//!
//! Each test builds its call context BY HAND (minimal defaults); the `FILL IN`
//! comments mark where real parachain inputs go.

#[cfg(test)]
mod tests {
    use jam_node::PvmBackend;
    use jam_node::vm::{
        AccumulateCallContext, Engine, RefineCallContext, RefineCallContextOwned, StateMutations,
        Storage,
    };
    use jam_std_common::{Entropy, Privileges, Service, hash_raw};
    use jam_types::{
        AccumulateItem, Authorizer, CodeHash, FixedVec, Hash, RefineContext, ServiceId, WorkItem,
        WorkItemRecord, WorkOutput, WorkPackage, WorkPayload,
    };

    /// Service id the blob is registered under (funded so storage host calls pass).
    const SERVICE_ID: ServiceId = 0;
    const GAS: u64 = 5_000_000_000;

    /// The service blob built by `just build` (`target/parachain-service.jam`).
    fn service_blob() -> Vec<u8> {
        blob("parachain-service.jam")
    }

    /// The authorizer blob built by `just build` (`target/parachain-authorizer.jam`).
    fn authorizer_blob() -> Vec<u8> {
        blob("parachain-authorizer.jam")
    }

    /// Read a built PVM blob `name` from the repo `target/` dir.
    fn blob(name: &str) -> Vec<u8> {
        let path = format!("{}/../../target/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("reading {path}: {e}\nBuild it first with `just build`."))
    }

    /// In-memory storage with `blob` registered + provided as `SERVICE_ID`'s code,
    /// under a generously funded account so storage-touching host calls succeed.
    fn storage_with_code(blob: &[u8]) -> (Storage, Hash) {
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
        storage.solicit(0, SERVICE_ID, code_hash, blob.len() as u32).expect("solicit");
        storage.provide(0, SERVICE_ID, blob).expect("provide");
        storage.commit();
        (storage, code_hash)
    }

    /// Interpreter-backed engine (deterministic for debugging).
    fn interpreter_engine() -> Engine {
        Engine::new(Some(PvmBackend::Interpreter)).expect("engine")
    }

    fn work_item(code_hash: Hash) -> WorkItem {
        WorkItem {
            service: SERVICE_ID,
            code_hash: CodeHash(code_hash),
            refine_gas_limit: GAS,
            accumulate_gas_limit: 0,
            export_count: 0,
            payload: WorkPayload(Vec::new()),
            import_segments: Default::default(),
            extrinsics: Default::default(),
        }
    }

    fn work_package(work_item: WorkItem, authorizer_code_hash: Hash) -> WorkPackage {
        WorkPackage {
            authorization: Default::default(),
            auth_code_host: SERVICE_ID,
            authorizer: Authorizer { code_hash: CodeHash(authorizer_code_hash), config: Default::default() },
            context: RefineContext {
                anchor: Default::default(),
                state_root: Default::default(),
                beefy_root: Default::default(),
                lookup_anchor: Default::default(),
                lookup_anchor_slot: 0,
                prerequisites: Default::default(),
            },
            items: vec![work_item].try_into().expect("within work-item bound"),
        }
    }

    #[test]
    fn refine_runs() {
        let code = service_blob();
        let (storage, code_hash) = storage_with_code(&code);
        let work_item = work_item(code_hash);

        let auth_code = authorizer_blob();
        let auth_code_hash = hash_raw(&auth_code);
        let work_package = work_package(work_item, auth_code_hash);

        let mut ctx = RefineCallContextOwned {
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
            work_item_index: 0,
            output_len: 0,
            engine: interpreter_engine(),
        };

        let engine = interpreter_engine();
        let (res, elapsed, gas_used) =
            engine.refine(CodeHash(code_hash), RefineCallContext::from(&mut ctx));
        let output = res.expect("refine should run to completion (not trap)");
        println!(
            "refine ok in {elapsed:?}, gas used {gas_used}, output: {} bytes",
            output.0.len()
        );
    }

    // Traps in the guest's own accumulate logic on the empty `WorkItemRecord`.
    // Un-ignore once the `FILL IN` items below carry real inputs.
    #[test]
    #[ignore]
    fn accumulate_runs() {
        let blob = service_blob();
        let (storage, code_hash) = storage_with_code(&blob);

        // --- FILL IN: the accumulation inputs (one empty work-item result now). ---
        let items = vec![AccumulateItem::WorkItem(WorkItemRecord {
            package: Default::default(),
            exports_root: Default::default(),
            authorizer_hash: Default::default(),
            payload: Default::default(),
            gas_limit: 0,
            result: Ok(WorkOutput(Vec::new())),
            auth_output: Default::default(),
        })];

        let mut ctx = AccumulateCallContext {
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

        let engine = interpreter_engine();
        let (res, elapsed, gas_used) = engine.accumulate(CodeHash(code_hash), &mut ctx);
        let yielded = res.expect("accumulate should run to completion (not trap)");
        println!("accumulate ok in {elapsed:?}, gas used {gas_used}, yielded: {yielded:?}");
    }
}
