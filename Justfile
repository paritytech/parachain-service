mod parachain 'scripts/parachain.justfile'
mod service 'scripts/service.justfile'

# Shared paths & config; also imported by the modules above.
import 'scripts/common.justfile'

default: help

help:
	just --list

# Run all checks
ci: fmt-check check build lint

# Fetch the vendored dependencies a fresh clone needs. See VENDOR.md.
vendor:
	git submodule update --init

# Short for fmt
f: fmt
# Format the code
fmt:
	#!/usr/bin/env sh
	if command -v cargo-ff >/dev/null 2>&1; then # https://github.com/ggwpez/cargo-ff
		cargo +nightly ff --all
	else
		cargo +nightly fmt --all
	fi

# Verify formatting without touching the tree (used by `ci`).
fmt-check:
	cargo +nightly fmt --all --check

# Short for check
c: check
check:
	cargo check --all-targets --workspace

# Remove target dir
clean:
	rm -rf target

# Short for build
b: build
# Build the native and PVM code
build: build-native build-pvm

build-native:
	SKIP_WASM_BUILD=1 cargo build

# Build the service, authorizer and parachain-runtime PVM blobs
build-pvm: build-service-pvm build-authorizer-pvm build-runtime-pvm

build-service-pvm:
	#!/usr/bin/env sh
	set -eu

	mkdir -p target
	jam-pvm-build --module service --output {{ SERVICE_BLOB }} service

	just check-blob-size {{ SERVICE_BLOB }}

build-authorizer-pvm:
	#!/usr/bin/env sh
	set -eu

	mkdir -p target
	jam-pvm-build --module authorizer --output {{ AUTHORIZER_BLOB }} authorizer

	just check-blob-size {{ AUTHORIZER_BLOB }}

build-runtime-pvm:
	#!/usr/bin/env sh
	set -eu
	mkdir -p target

	# TODO: build with the `production` profile instead of `--release`.
	SUBSTRATE_RUNTIME_TARGET=riscv cargo build --release --package frameless

	just check-blob-size {{ FRAMELESS_BLOB }}

lint:
	cargo clippy --all-targets --workspace

# Run the full workspace test suite
test:
	cargo test --all-targets --workspace

# Fail if the given blob exceeds MAX_BLOB_SIZE.
check-blob-size blob:
	#!/usr/bin/env sh
	set -eu
	size=$(wc -c < "{{ blob }}")
	if [ "$size" -gt {{ MAX_BLOB_SIZE }} ]; then
		echo "{{ blob }}: $size bytes exceeds max {{ MAX_BLOB_SIZE }}" >&2
		exit 1
	fi
