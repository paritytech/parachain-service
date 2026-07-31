mod parachain 'scripts/parachain.justfile'
mod service 'scripts/service.justfile'

# Shared paths & config; also imported by the modules above.
import 'scripts/common.justfile'

default: help

help:
	just --list

# Run all checks
ci: fmt check build lint

# Fetch the build-critical vendored dependencies a fresh clone needs.
vendor:
	#!/usr/bin/env sh
	set -eu
	# The `polkadot-sdk-companion` submodule plus the (non-submodule, pinned)
	# polkajam checkout that `tools/executor` builds against. Reference-only
	# vendors (graypaper, cumulus/dafny specs) are cloned manually — see CLAUDE.md.
	git submodule update --init --recursive
	if [ ! -d vendor/polkajam/.git ]; then
		git clone {{ POLKAJAM_URL }} vendor/polkajam
	fi
	git -C vendor/polkajam fetch --quiet origin {{ POLKAJAM_REV }} 2>/dev/null || git -C vendor/polkajam fetch --quiet origin
	git -C vendor/polkajam checkout --quiet {{ POLKAJAM_REV }}
	echo "vendored: polkajam @ {{ POLKAJAM_REV }} + submodules"

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
	# The `executor` runtime/service backends and the `executor-cli` binary sit
	# behind non-default features, so `--workspace` alone never compiles them
	# (the `jam` backend is covered above via the service/asset-hub test deps).
	# Check them explicitly, else CI stays green while that code rots.
	cargo check --all-targets --package executor-cli --features executor

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
	# See `check`: lint the feature-gated executor backends and CLI too.
	cargo clippy --all-targets --package executor-cli --features executor

check-blob-sizes blob:
	#!/usr/bin/env sh
	set -eu
	if [ $(wc -c < "{{ blob }}") -gt {{ MAX_BLOB_SIZE }} ]; then
		echo "  ^ too large (max {{ MAX_BLOB_SIZE }} bytes)"
		exit 1
	fi
