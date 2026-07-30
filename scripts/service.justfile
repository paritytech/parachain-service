import 'common.justfile'

# `refine`/`accumulate` run the blob on our own PolkaVM 0.36 run-loop with the
# log-and-abort debug host. `just service test` uses the real polkajam node host.

# Run the service `refine` entry point on the debug stub host.
refine *ARGS:
	cargo run --package {{ EXECUTOR_PACKAGE }} --features executor -- service --blob {{ SERVICE_BLOB }} refine {{ ARGS }}

# Run the service `accumulate` entry point on the debug stub host.
accumulate *ARGS:
	cargo run --package {{ EXECUTOR_PACKAGE }} --features executor -- service --blob {{ SERVICE_BLOB }} accumulate {{ ARGS }}

# Run the refine/accumulate tests against the *real node host* on the built blob.
# `RUST_LOG=jam_node=trace,pvm=trace` + `-- --include-ignored --nocapture`
# shows the host-call trace.
test *ARGS:
	cargo test --package parachain-service {{ ARGS }}
