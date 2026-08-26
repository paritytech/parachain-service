#!/usr/bin/env bash
# Build the parasim service blob, register it on a running polkajam testnet, and
# submit mock work packages whose heads you then read back — a scripted tour of
# refine -> accumulate -> set_storage.
#
# Prereqs:
#   - a polkajam testnet node with RPC at ${JAM_RPC:-ws://localhost:19800}
#   - `jamt` on PATH (build it in ./polkajam),
#   - the parasim blob buildable in this workspace (see the .agent worklog note
#     on vendor submodules; on a networked checkout this is just `cargo build --release`).
#
# Usage: scripts/parasim-run.sh [num_heads [service_id]]
#   num_heads   how many heads to push (default 3)
#   service_id  pin the parasim service id (default 5; must be unused, < 65536)

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
NUM_HEADS="${1:-3}"
SERVICE_ID="${2:-5}"

cd "$ROOT"

# 1. Build the parasim blob and the payload-generating sender.
cargo build -p parasim-service-bin --release
cargo build -p parasim-send --release
BLOB="$(find target -type f -name 'parasim-service.jam' | head -n1)"
test -n "$BLOB"
SENDER="target/release/parasim-send"
JAMT="${JAMT:-jamt}"

# 2. Register the service and capture its id (--raw prints only the hex id).
NEW_ID="$("$JAMT" create-service "$BLOB" 0 --register=parasim --raw --id "$SERVICE_ID" 2>/dev/null)"
echo "parasim service id: $NEW_ID"

# The dev-genesis null authorizer (empty config) makes refine fall back to
# `FALLBACK_PARA_ID` (0), so the head is always stored at para 0. Its key is
# the storage tag 0x00 + SCALE(ParaId(0) as LE u32) = 0x0000000000.
KEY=0x0000000000

# 3. Push a few heads and read each back after accumulate has had a moment.
for n in $(seq "$NUM_HEADS"); do
	PAYLOAD="$("$SENDER" --number "$n")"
	echo "submitting head #$n"
	"$JAMT" item "$NEW_ID" "$PAYLOAD" --force-core 0
	sleep 2   # allow refine + accumulate to land
	HEAD="$( "$JAMT" service storage "$NEW_ID" "$KEY" --raw 2>/dev/null || echo "<pending>")"
	echo "head after #$n: $HEAD"
done