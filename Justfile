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

	# Use cargo-ff, if available: https://github.com/ggwpez/cargo-ff
	if command -v cargo-ff >/dev/null 2>&1; then
		cargo +nightly ff --all
	else
		cargo +nightly fmt --all
	fi

# Short for check
c: check
check:
	cargo check --all-targets --workspace

# Short for build
b: build
# Build the native and PVM code
build: build-native build-pvm

build-native:
	SKIP_WASM_BUILD=1 cargo build

# Build the service and parachain-runtime PVM blobs
build-pvm: build-service-pvm build-runtime-pvms

build-service-pvm:
	mkdir -p target
	jam-pvm-build --module service --output target/parachain-service.jam service

build-runtime-pvms:
	mkdir -p target
	SUBSTRATE_RUNTIME_TARGET=riscv cargo build --release --package asset-hub --package coretime
	@echo ""
	@echo "Asset Hub PVM blob: target/release/rbuild/asset-hub/asset-hub-blob.polkavm"
	@echo "Coretime PVM blob:  target/release/rbuild/coretime/coretime-blob.polkavm"

lint:
	cargo clippy --all-targets --workspace

# TODO: use production profile for building
