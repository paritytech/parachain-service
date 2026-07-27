import 'common.justfile'

# Run the service `refine` entry point on a bare PolkaVM (JAM host calls log & abort).
# Extra ARGS are forwarded, e.g. `--input refine.json` or `--gas 1000000`.
refine *ARGS:
	cargo run --manifest-path {{ EXECUTOR }} -- service --blob {{ SERVICE_BLOB }} refine {{ ARGS }}

# Run the service `accumulate` entry point on a bare PolkaVM.
accumulate *ARGS:
	cargo run --manifest-path {{ EXECUTOR }} -- service --blob {{ SERVICE_BLOB }} accumulate {{ ARGS }}
