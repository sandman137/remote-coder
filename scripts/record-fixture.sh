#!/usr/bin/env bash
# Record a raw control-mode stream from a fake agent into
# fixtures/control-mode/<name>.stream for golden tests (DESIGN.md §10.3).
#
# Usage: scripts/record-fixture.sh <name> [agent-script] [seconds]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME="${1:?usage: record-fixture.sh <name> [agent-script] [seconds]}"
AGENT="${2:-$ROOT/fixtures/agents/fake-stream.sh}"
SECS="${3:-4}"
SOCK="helm-record-$$"
OUT="$ROOT/fixtures/control-mode/$NAME.stream"

cleanup() { tmux -L "$SOCK" kill-server 2>/dev/null || true; }
trap cleanup EXIT

tmux -L "$SOCK" -f /dev/null new-session -d -s rec -x 80 -y 24 "$AGENT"
# Hold stdin open (but silent) for the capture window; EOF detaches the client.
tmux -L "$SOCK" -C attach-session -t rec < <(sleep "$SECS") > "$OUT" || true

echo "wrote $OUT ($(wc -c < "$OUT") bytes)"
