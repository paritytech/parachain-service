#!/usr/bin/env bash
# Build the parasim service blob, register it on a running polkajam testnet, and
# submit mock work packages whose heads you then read back — a scripted tour of
# refine -> accumulate -> set_storage.
#
# Prereqs:
#   - a polkajam testnet node with RPC at ${JAM_RPC:-ws://localhost:19800}
#   - a built `jamt` binary at the default path below (build it in
#     ~/parity/46-jam-cummulus-side-2/polkajam), or override via JAMT,
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
# Endowment funding the service's state balance, which pays for set_storage.
# High enough to run for a very long time (each 98-byte head costs ~100).
ENDOWMENT="${ENDOWMENT:-1000000000000000000}"

cd "$ROOT"

# 1. Build the parasim blob and the payload-generating sender.
cargo build -p parasim-service-bin --release
cargo build -p parasim-send --release
BLOB="$(find target -type f -name 'parasim-service.jam' | head -n1)"
test -n "$BLOB"
SENDER="target/release/parasim-send"
JAMT="${JAMT:-/home/miszka/parity/46-jam-cummulus-side-2/polkajam/target/release/jamt}"

# 2. Register the service with a real endowment and capture its id (--raw prints
# only the hex id). Without the endowment set_storage fails ("Balance too low").
NEW_ID="$("$JAMT" create-service "$BLOB" "$ENDOWMENT" --register=parasim --raw --id "$SERVICE_ID" 2>/dev/null)"
echo "parasim service id: $NEW_ID"

# The dev-genesis null authorizer (empty config) makes refine fall back to
# `FALLBACK_PARA_ID` (0), so the head is always stored at para 0. Its key is
# the storage tag 0x00 (Tag::Parachains) + SCALE(ParaId(0) as LE u32) = 0x0000000000.
KEY=0x0000000000

# 3. Push a few heads and read each back after accumulate has had a moment.
# jamt item wants the payload as a 0x-prefixed hex string (else it sends the hex
# as literal ASCII), and --force-core is a global flag before the subcommand.
for n in $(seq "$NUM_HEADS"); do
	PAYLOAD="0x$("$SENDER" --number "$n" | grep -A1 '# work-item payload' | tail -1)"
	echo "submitting head #$n"
	"$JAMT" --force-core 0 item "$NEW_ID" "$PAYLOAD"
	sleep 12   # allow refine + accumulate + set_storage to land (a slot is ~6s)
	HEAD="$( "$JAMT" inspect storage "$NEW_ID" "$KEY" --raw 2>/dev/null || echo "<pending>")"
	echo "head after #$n: $HEAD"
done
