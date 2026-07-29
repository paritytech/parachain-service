import 'common.justfile'

# `refine`/`accumulate` run the blob on our own PolkaVM 0.36 run-loop + the
# log-and-abort stub host (src/service.rs, src/host.rs). To instead run against
# the *real* polkajam node host (Storage + full host calls), use `just service
# test` — that path lives in tests (src/jam_backend.rs) and pulls in jam-node.

# Run the service `refine` entry point on the debug stub host.
refine *ARGS:
	cargo run --manifest-path {{ EXECUTOR }} -- service --blob {{ SERVICE_BLOB }} refine {{ ARGS }}

# Run the service `accumulate` entry point on the debug stub host.
accumulate *ARGS:
	cargo run --manifest-path {{ EXECUTOR }} -- service --blob {{ SERVICE_BLOB }} accumulate {{ ARGS }}

# Run the refine/accumulate tests against the *real node host* on the built blob.
# jam-node needs `RUSTC_BOOTSTRAP=1`; `RUST_LOG=jam_node=trace,pvm=trace` +
# `-- --include-ignored --nocapture` shows the host-call trace.
test *ARGS:
	RUSTC_BOOTSTRAP=1 cargo test --manifest-path {{ EXECUTOR }} jam_backend {{ ARGS }}
