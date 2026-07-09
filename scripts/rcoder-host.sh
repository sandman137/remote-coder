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
# The phone needs nothing but the app. It dials whatever address ends up in
# the QR, picked in this order:
#   1. RC_HOST=<addr>          explicit override
#   2. the box's own public IP (cloud/VPS — detected + verified bound locally)
#   3. --via user@cloudbox     laptop mode: reverse-tunnels the broker through
#                              any box you own; the QR carries that box's addr
#   4. Tailscale IP            fallback if you prefer a tailnet
#
# Prereqs: tmux, openssh, and this repo built (`cargo build --release
# -p engine-cli -p broker`). For --via, the cloud box's sshd needs
# `GatewayPorts clientspecified` in /etc/ssh/sshd_config.
#
# Usage:
#   scripts/rcoder-host.sh                      # cloud box: public IP, port 8022
#   scripts/rcoder-host.sh --via me@vps.example # laptop behind NAT
#   RC_SESSION=work RC_PORT=9022 scripts/rcoder-host.sh
#   scripts/rcoder-host.sh --revoke <device>    # unpair a phone
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SESSION="${RC_SESSION:-agents}"
PORT="${RC_PORT:-8022}"
STATE="$ROOT/.rcoder"
VIA=""
mkdir -p "$STATE"; chmod 700 "$STATE"

# Prefer release binaries, fall back to debug.
bin() { [ -x "$ROOT/target/release/$1" ] && echo "$ROOT/target/release/$1" || echo "$ROOT/target/debug/$1"; }
RCODER="$(bin rcoder)"; BROKER="$(bin broker)"
[ -x "$RCODER" ] || { echo "build first:  cargo build --release -p engine-cli -p broker"; exit 1; }

case "${1:-}" in
  --revoke)
    "$RCODER" revoke "${2:?usage: --revoke <device>}" --authorized-keys "$STATE/authorized_keys"
    echo "revoked. It stops working on the phone's next reconnect."
    exit 0 ;;
  --via)
    VIA="${2:?usage: --via user@cloudbox[ , needs GatewayPorts clientspecified on that box]}" ;;
esac

command -v tmux >/dev/null || { echo "tmux is required (brew install tmux / apt install tmux)"; exit 1; }
SSHD_BIN="$(command -v sshd || echo /usr/sbin/sshd)"
[ -x "$SSHD_BIN" ] || { echo "sshd not found (install openssh-server)"; exit 1; }

# ---- pick the address the phone will dial --------------------------------
have_ip_bound() {  # is $1 one of this box's own addresses?
  { ip -4 addr show 2>/dev/null || ifconfig 2>/dev/null; } | grep -q "inet $1[ /]"
}
MODE=""; ADDR=""
PUB="$(curl -4 -s --max-time 4 ifconfig.me 2>/dev/null || true)"
if [ -n "${RC_HOST:-}" ]; then
  ADDR="$RC_HOST"; MODE="explicit (RC_HOST)"
elif [ -n "$VIA" ]; then
  ADDR="${VIA#*@}"; ADDR="${ADDR%%:*}"; MODE="via $VIA (reverse tunnel)"
elif [ -n "$PUB" ] && have_ip_bound "$PUB"; then
  ADDR="$PUB"; MODE="direct public IP"
elif TSIP="$(tailscale ip -4 2>/dev/null | head -1)" && [ -n "${TSIP:-}" ]; then
  ADDR="$TSIP"; MODE="tailnet (phone must run Tailscale too)"
else
  cat <<EOF
Can't find an address the phone could reach:
  - this box has no public IP bound (behind NAT?)
  - no --via cloudbox given
  - no tailnet
On a laptop, use:  $0 --via user@your-cloud-box
EOF
  exit 1
fi

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
  echo "started broker sshd on :$PORT (pubkey-only, forced-command)"
fi

# --- laptop mode: reverse-tunnel broker + enroll ports through the cloud box
if [ -n "$VIA" ]; then
  EPORT=$((PORT + 1))
  if ! pgrep -f "ssh .*-R.*$PORT:localhost:$PORT.*$VIA" >/dev/null 2>&1; then
    ssh -f -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 \
      -R "0.0.0.0:$PORT:localhost:$PORT" \
      -R "0.0.0.0:$EPORT:localhost:$EPORT" \
      "$VIA"
    echo "reverse tunnel up: $ADDR:$PORT -> this machine"
  fi
  # verify the tunnel is reachable from outside the laptop
  if command -v nc >/dev/null && ! nc -z -w4 "$ADDR" "$PORT" 2>/dev/null; then
    cat <<EOF
!! $ADDR:$PORT is not reachable — the tunnel bound to loopback only.
   On $ADDR, add to /etc/ssh/sshd_config and restart sshd:
       GatewayPorts clientspecified
EOF
    exit 1
  fi
fi

echo
echo "════════════════════════════════════════════════════════════════"
echo " Remote Coder host ready:  session '$SESSION'"
echo " address: $ADDR:$PORT   ($MODE)"
echo " Open the app on your phone → Scan QR ↓"
echo "════════════════════════════════════════════════════════════════"
echo

# --- pairing: prints the QR, enrolls one phone, then exits ---
exec "$RCODER" pair \
  --pair-host "$ADDR" --ssh-port "$PORT" \
  --authorized-keys "$STATE/authorized_keys" \
  --broker-path "$BROKER" \
  --scope-session "$SESSION" \
  --enroll-port "$((PORT + 1))" --ttl 600 \
  --hostkey-pub "$STATE/host_ed25519.pub"
