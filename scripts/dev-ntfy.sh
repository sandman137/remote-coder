#!/usr/bin/env bash
# Local ntfy server for notification tests (DESIGN.md §9, §10.5).
# Prefers a local `ntfy` binary, falls back to docker. Foreground; Ctrl-C stops.
set -euo pipefail

PORT="${HELM_NTFY_PORT:-2586}"

if command -v ntfy >/dev/null 2>&1; then
  echo "ntfy serving on http://127.0.0.1:$PORT"
  exec ntfy serve --listen-http ":$PORT" --base-url "http://127.0.0.1:$PORT"
elif command -v docker >/dev/null 2>&1; then
  echo "ntfy (docker) serving on http://127.0.0.1:$PORT"
  exec docker run --rm -p "127.0.0.1:$PORT:80" binwiederhier/ntfy serve
else
  echo "error: neither ntfy nor docker found — install one of them" >&2
  echo "  https://docs.ntfy.sh/install/" >&2
  exit 1
fi
