#!/usr/bin/env bash
# Remote Coder — dev-host launcher.
#
# Stands up everything the phone needs, on the machine where your coding
# agents run:
#   • a tmux session (default: "agents") you run Claude Code / Codex / Cursor in
#   • an SSH endpoint whose only capability is the Remote Coder broker,
#     scoped to that session (a paired phone key can NEVER get a shell)
#   • a one-time pairing QR
#
# Prereqs: tmux, ssh (openssh), and this repo built (`cargo build --release
# -p engine-cli -p broker`). The host must be on your tailnet; so must the
# phone (install the Tailscale app and sign in to the same tailnet).
#
# Usage:
#   scripts/rcoder-host.sh                       # session "agents", port 8022
#   RC_SESSION=work RC_PORT=8022 scripts/rcoder-host.sh
#   scripts/rcoder-host.sh --revoke <device>     # unpair a phone
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SESSION="${RC_SESSION:-agents}"
PORT="${RC_PORT:-8022}"
STATE="$ROOT/.rcoder"
mkdir -p "$STATE"; chmod 700 "$STATE"

# Prefer release binaries, fall back to debug.
bin() { [ -x "$ROOT/target/release/$1" ] && echo "$ROOT/target/release/$1" || echo "$ROOT/target/debug/$1"; }
RCODER="$(bin rcoder)"; BROKER="$(bin broker)"
[ -x "$RCODER" ] || { echo "build first:  cargo build --release -p engine-cli -p broker"; exit 1; }

# --- revoke path ---
if [ "${1:-}" = "--revoke" ]; then
  "$RCODER" revoke "${2:?usage: --revoke <device>}" --authorized-keys "$STATE/authorized_keys"
  echo "revoked. It stops working on the phone's next reconnect."
  exit 0
fi

command -v tmux >/dev/null || { echo "tmux is required (brew install tmux / apt install tmux)"; exit 1; }
SSHD_BIN="$(command -v sshd || echo /usr/sbin/sshd)"
[ -x "$SSHD_BIN" ] || { echo "sshd not found (install openssh-server)"; exit 1; }

# Tailnet IP (the address the phone dials).
TSIP="$(tailscale ip -4 2>/dev/null | head -1 || true)"
[ -n "$TSIP" ] || { echo "not on a tailnet — start Tailscale on this host first"; exit 1; }

# --- host key (persistent per checkout) ---
[ -f "$STATE/host_ed25519" ] || ssh-keygen -q -t ed25519 -N '' -C rcoder-host -f "$STATE/host_ed25519"

# --- ensure the agents tmux session exists (yours to fill with agents) ---
if ! tmux has-session -t "=$SESSION" 2>/dev/null; then
  tmux new-session -d -s "$SESSION" -x 100 -y 30
  tmux set-option -t "=$SESSION" -g window-size latest
  echo "created empty tmux session '$SESSION' — attach and launch your agents:"
  echo "    tmux attach -t $SESSION      # then run: claude   (or codex / cursor 'agent')"
fi

# --- dedicated broker-only sshd (does NOT touch your ~/.ssh) ---
touch "$STATE/authorized_keys"; chmod 600 "$STATE/authorized_keys"
cat > "$STATE/sshd_config" <<EOF
Port $PORT
ListenAddress 0.0.0.0
HostKey $STATE/host_ed25519
PidFile $STATE/sshd.pid
AuthorizedKeysFile $STATE/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
PermitRootLogin no
StrictModes no
UsePAM no
AllowUsers $USER
EOF

if ! (ss -ltn 2>/dev/null || netstat -an 2>/dev/null) | grep -q "[:.]$PORT "; then
  "$SSHD_BIN" -f "$STATE/sshd_config"
  echo "started broker sshd on $TSIP:$PORT"
fi

echo
echo "════════════════════════════════════════════════════════════════"
echo " Remote Coder host ready:  session '$SESSION'  ·  $TSIP:$PORT"
echo " Open the app on your phone → Scan QR ↓  (phone must be on this tailnet)"
echo "════════════════════════════════════════════════════════════════"
echo

# --- pairing: prints the QR, enrolls one phone, then exits ---
exec "$RCODER" pair \
  --pair-host "$TSIP" --ssh-port "$PORT" \
  --authorized-keys "$STATE/authorized_keys" \
  --broker-path "$BROKER" \
  --scope-session "$SESSION" \
  --enroll-port "$((PORT + 1))" --ttl 600 \
  --hostkey-pub "$STATE/host_ed25519.pub"
