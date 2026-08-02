import 'common.justfile'

# Run the refine/accumulate tests against the *real node host* on the built blob.
# `RUST_LOG=jam_node=trace,pvm=trace` + `-- --include-ignored --nocapture`
# shows the host-call trace.
test *ARGS:
	cargo test --package parachain-service {{ ARGS }}
