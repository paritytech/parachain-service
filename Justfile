mod parachain 'scripts/parachain.justfile'
mod service 'scripts/service.justfile'

# Shared paths & config; also imported by the modules above.
import 'scripts/common.justfile'

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
		cargo +nightly ff \
			--package asset-hub \
			--package asset-hub-system \
			--package coretime \
			--package coretime-system \
			--package executor \
			--package executor-cli \
			--package parachain-authorizer \
			--package parachain-service
	else
		cargo +nightly fmt \
			--package asset-hub \
			--package asset-hub-system \
			--package coretime \
			--package coretime-system \
			--package executor \
			--package executor-cli \
			--package parachain-authorizer \
			--package parachain-service
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

# Build the service, authorizer and parachain-runtime PVM blobs
build-pvm: build-service-pvm build-authorizer-pvm build-runtime-pvms

build-service-pvm:
	#!/usr/bin/env sh
	set -eu

	mkdir -p target
	jam-pvm-build --module service --output {{ SERVICE_BLOB }} service

	just check-blob-sizes {{ SERVICE_BLOB }}

build-authorizer-pvm:
	#!/usr/bin/env sh
	set -eu

	mkdir -p target
	jam-pvm-build --module authorizer --output {{ AUTHORIZER_BLOB }} authorizer

	just check-blob-sizes {{ AUTHORIZER_BLOB }}

build-runtime-pvms:
	#!/usr/bin/env sh
	set -eu
	mkdir -p target

	SUBSTRATE_RUNTIME_TARGET=riscv cargo build --release --package asset-hub --package coretime

	just check-blob-sizes {{ ASSET_HUB_BLOB }}
	just check-blob-sizes {{ CORETIME_BLOB }}

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
