mod parachain 'scripts/parachain.justfile'

AHM_PVM_BLOB := "target/release/rbuild/asset-hub/asset-hub-blob.polkavm"
CORETIME_PVM_BLOB := "target/release/rbuild/coretime/coretime-blob.polkavm"
SERVICE_PVM_BLOB := "target/parachain-service.jam"

# Max size of Service and para runtime blobs
MAX_BLOB_SIZE := "4194304"

default: help

help:
	just --list

# Run all checks
ci: fmt check build lint

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

# Build the service and parachain-runtime PVM blobs
build-pvm: build-service-pvm build-runtime-pvms

build-service-pvm:
	#!/usr/bin/env sh
	set -eu

	mkdir -p target
	jam-pvm-build --module service --output {{ SERVICE_PVM_BLOB }} service

	just check-blob-sizes {{ SERVICE_PVM_BLOB }}

build-runtime-pvms:
	#!/usr/bin/env sh
	set -eu
	mkdir -p target

	SUBSTRATE_RUNTIME_TARGET=riscv cargo build --release --package asset-hub --package coretime

	just check-blob-sizes {{ AHM_PVM_BLOB }}
	just check-blob-sizes {{ CORETIME_PVM_BLOB }}

# TODO: use production profile for building

lint:
	cargo clippy --all-targets --workspace

check-blob-sizes blob:
	#!/usr/bin/env sh
	set -eu
	if [ $(wc -c < "{{ blob }}") -gt {{ MAX_BLOB_SIZE }} ]; then
		echo "  ^ too large (max {{ MAX_BLOB_SIZE }} bytes)"
		exit 1
	fi
