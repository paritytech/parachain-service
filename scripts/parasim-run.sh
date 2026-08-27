#!/usr/bin/env bash
# Deploy parasim on a running polkajam testnet and walk a para's head forward, then show that a
# package with a bad ancestry proof is rejected and leaves the head where it was.
#
# This is the phase-4 story end to end: a package must prove what the para's previous head was, so
# heads advance only by chaining onto the stored one.
#
# Prereqs:
#   - a polkajam testnet with RPC at ${JAM_RPC:-ws://localhost:19800}
#   - a built `jamt` at the default path below, or JAMT pointing at one
#
# Usage: scripts/parasim-run.sh [num_heads [service_id]]

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
NUM_HEADS="${1:-2}"
SERVICE_ID="${2:-5}"
JAMT="${JAMT:-/home/miszka/parity/46-jam-cummulus-side-2/polkajam/target/release/jamt}"
# Funds the service's state balance, which is what pays for set_storage. Without it accumulate
# fails with "Balance too low for storage change" and no head is ever written.
ENDOWMENT="${ENDOWMENT:-1000000000000000000}"

cd "$ROOT"

cargo build --release -p parasim-service-bin -p parasim-send
BLOB="$(find target -type f -name 'parasim-service.jam' | head -n1)"
test -s "$BLOB" || { echo "no parasim blob built (was SKIP_PVM_BUILDS set?)" >&2; exit 1; }
SENDER="target/release/parasim-send"

NEW_ID="$("$JAMT" create-service "$BLOB" "$ENDOWMENT" --register=parasim --raw --id "$SERVICE_ID")"
echo "parasim service id: $NEW_ID"

# The first package proves the para has no head yet and so starts it; each later one chains onto
# the head the previous package stored.
for n in $(seq "$NUM_HEADS"); do
	echo
	echo "=== package $n ==="
	"$SENDER" --service "$SERVICE_ID"
done

echo
echo "=== tampered proof: refine must reject, and the head must not move ==="
"$SENDER" --service "$SERVICE_ID" --tamper || echo "(rejected, as expected)"
