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
	cargo build

# Build the three PVM blobs: service, asset-hub, and coretime
build-pvm:
	mkdir -p target
	jam-pvm-build --module service --output target/parachain-service.jam service
	jam-pvm-build --module asset-hub --output target/asset-hub.jam asset-hub
	jam-pvm-build --module coretime --output target/coretime.jam coretime # TODO use production profile

lint:
	cargo clippy --all-targets --workspace
