#!/usr/bin/env bash
# 
# Generic `RUSTC_WRAPPER` that makes cargo builds cross-host reproducible at the
# symbol-mangling level. Replaces cargo's per-crate `-Cmetadata` value (which
# embeds workspace path and other host-dependent inputs) with a fixed string
# derived from the crate name, a hash of the crate's `Cargo.toml`, and the
# target triple. With this, rustc's `StableCrateId` is identical on Linux and
# macOS.
#
# Not PVM-specific. Use by exporting `RUSTC_WRAPPER` before running cargo, e.g.
#   RUSTC_WRAPPER=$(pwd)/crates/jam-pvm-builder/scripts/rustc-wrapper.sh cargo build …
#
# Invocation contract: cargo runs us as `wrapper <rustc-path> <rustc-args...>`.
# Output filename naming via `--extra-filename` is left untouched so cargo's
# on-disk path expectations stay consistent.
#
# Caveat: toggling `RUSTC_WRAPPER` on and off against the same `target/`
# directory can leave rustc's incremental cache inconsistent with the rlibs on
# disk, manifesting as an ICE like:
#   error: internal compiler error: uninterned StableCrateId(...)
# Recover with `rm -rf target/debug/incremental` (cheap) or `cargo clean`.
 
set -eu

# Future-edit insurance: this script must always end by `exec "$rustc" …`, replacing the shell
# with rustc. Under `set -eu` plus the current structure (every branch ends in `exec`) this is
# already guaranteed, but it is one accidental edit away from breaking — and when it breaks the
# failure mode is silent: no rustc invoked, shell exits 0, cargo caches the empty result as a
# successful compile. That once silently poisoned proc-macro2's nightly probe, producing E0554
# errors on stable rustc that survived across rebuilds via cached build-script output. The trap
# fails loudly if any future branch forgets to `exec`, so the bug is caught at the source
# instead of being baked into target/. (It also fires if `exec` itself fails, which is fine —
# rustc-not-found is exactly the kind of failure we want surfaced with a clear message.)
trap 'echo "[wrapper] BUG: exited without exec rustc" >&2; exit 99' EXIT

# Print a debug line when `WRAPPER_DEBUG` is set in the environment. By default writes to
# stderr; set `WRAPPER_DEBUG_LOG=/abs/path/to/file` to capture into a file instead (more
# reliable when stderr gets buffered or filtered by cargo). Activate with:
#   WRAPPER_DEBUG=1 WRAPPER_DEBUG_LOG=$(pwd)/wrapper.log \
#     RUSTC_WRAPPER=$(pwd)/crates/jam-pvm-builder/scripts/rustc-wrapper.sh cargo build ...
# Note: `WRAPPER_DEBUG_LOG` must be an absolute path. Cargo changes the working directory
# per rustc invocation, so a relative path resolves to a different file each time (usually
# inside a registry crate dir) and writes scatter or fail silently.
if [ -n "${WRAPPER_DEBUG_LOG:-}" ] && [ "${WRAPPER_DEBUG_LOG#/}" = "$WRAPPER_DEBUG_LOG" ]; then
	echo "[wrapper] WRAPPER_DEBUG_LOG must be an absolute path, got: $WRAPPER_DEBUG_LOG; falling back to stderr" >&2
	# Don't refuse to run — fall back to stderr so the build isn't blocked.
	unset WRAPPER_DEBUG_LOG
fi
debug() {
	[ -n "${WRAPPER_DEBUG:-}" ] || return 0
	if [ -n "${WRAPPER_DEBUG_LOG:-}" ]; then
		printf '[wrapper] %s\n' "$*" >>"$WRAPPER_DEBUG_LOG"
	else
		printf '[wrapper] %s\n' "$*" >&2
	fi
}

rustc="$1"
shift

crate_name=""
target=""
src_file=""
prev=""
for arg in "$@"; do
	if [ "$prev" = "--crate-name" ]; then
		crate_name="$arg"
	elif [ "$prev" = "--target" ]; then
		target="$arg"
	fi
	case "$arg" in
		--crate-name=*) crate_name="${arg#--crate-name=}" ;;
		--target=*) target="${arg#--target=}" ;;
		-*) ;;
		*.rs)
			# First positional `.rs` is rustc's crate root. The `-z` guard ensures we only
			# consume the first one — a later value-shaped arg (e.g. `--cfg foo.rs`) could
			# in principle also match this pattern, and we want to anchor on the crate root.
			[ -z "$src_file" ] && src_file="$arg"
			;;
	esac
	prev="$arg"
done

# `--target` may be a triple or a path to a target.json file. Reduce both to a
# stable basename so the wrapper's metadata is host-independent. Default to
# "host" when no target was given (proc-macros, build scripts).
if [ -n "$target" ]; then
	target="$(basename "${target%.json}")"
else
	target="host"
fi

# Introspection invocations (e.g. `rustc --version`, `--print sysroot`, `-vV`) carry no
# `--crate-name` and no `-Cmetadata`. Nothing to rewrite - pass through verbatim.
if [ -z "$crate_name" ]; then
	exec "$rustc" "$@"
fi

# Find the Cargo.toml that owns this crate by walking up from the crate root's directory.
# Stops at the first `Cargo.toml` found or at the filesystem root.
manifest=""
if [ -n "$src_file" ]; then
	d="$(dirname "$src_file")"
	while :; do
		if [ -f "$d/Cargo.toml" ]; then
			manifest="$d/Cargo.toml"
			break
		fi
		parent="$(dirname "$d")"
		[ "$parent" = "$d" ] && break
		d="$parent"
	done
fi

# Build a host-independent metadata key. Hashing Cargo.toml (rather than the crate root's
# path or its .rs contents) keys on the crate's actual identity: name, version, deps,
# features. Two builds of the same crate produce the same `StableCrateId` regardless of
# cargo's registry layout, workspace location, or username, and crates with near-empty
# crate roots (`pub mod foo;`) don't collide with each other. The crate name is folded
# in to disambiguate the build-script case where many crates share `--crate-name=
# build_script_build` and would otherwise share a hash if their Cargo.toml is identical.
# Uses `shasum -a 256` for portability — installed by default on both Linux and macOS.
if [ -n "$manifest" ] && [ -r "$manifest" ]; then
	manifest_hash="$(shasum -a 256 "$manifest" | cut -d ' ' -f 1 | cut -c1-16)"
	metadata_key="${crate_name}-${manifest_hash}-${target}"
else
	# Defensive: no Cargo.toml found (shouldn't happen for real compile invocations).
	metadata_key="${crate_name}-${target}"
fi

debug "========================================================================"
debug "crate_name:   $crate_name"
debug "manifest:     $manifest"
debug "metadata_key: $metadata_key"

# Rewrite the per-crate `-Cmetadata` value.
# Cargo emits it as either `-Cmetadata=X` or `-C metadata=X`; handle both.
args=()
all=("$@")
i=0
while [ $i -lt ${#all[@]} ]; do
	arg="${all[$i]}"
	next_idx=$((i + 1))
	if [ "$arg" = "-C" ] && [ $next_idx -lt ${#all[@]} ]; then
		next="${all[$next_idx]}"
		case "$next" in
			metadata=*)
				args+=("-C" "metadata=${metadata_key}")
				i=$((next_idx + 1))
				continue
				;;
		esac
	fi
	case "$arg" in
		-Cmetadata=*)
			args+=("-Cmetadata=${metadata_key}")
			;;
		*)
			args+=("$arg")
			;;
	esac
	i=$((i + 1))
done

exec "$rustc" "${args[@]}"
