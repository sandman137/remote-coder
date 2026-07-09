#!/usr/bin/env bash
# Loopback sshd on 127.0.0.1:2222 with a throwaway host key and test client
# key — proves the whole remote stack (auth, pinning, control-mode-over-SSH,
# broker) on one machine (DESIGN.md §3.2, §10 layer 4).
#
# Usage:
#   scripts/dev-sshd.sh            # authorized key runs commands directly
#   scripts/dev-sshd.sh --broker   # authorized key is forced through the broker (Phase 6)
#
# State lives in .dev/sshd/ (gitignored). Foreground; Ctrl-C to stop.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE="$ROOT/.dev/sshd"
PORT="${HELM_SSHD_PORT:-2222}"
USE_BROKER=0
[[ "${1:-}" == "--broker" ]] && USE_BROKER=1

SSHD_BIN="$(command -v sshd || echo /usr/sbin/sshd)"
if [[ ! -x "$SSHD_BIN" ]]; then
  echo "error: sshd not found — install openssh-server" >&2
  exit 1
fi

mkdir -p "$STATE"
chmod 700 "$STATE"

# Host key (throwaway, per checkout).
if [[ ! -f "$STATE/host_ed25519" ]]; then
  ssh-keygen -q -t ed25519 -N '' -C helm-dev-host -f "$STATE/host_ed25519"
fi

# Client test key.
if [[ ! -f "$STATE/client_ed25519" ]]; then
  ssh-keygen -q -t ed25519 -N '' -C helm-dev-client -f "$STATE/client_ed25519"
fi

# authorized_keys — optionally forced through the broker.
if [[ "$USE_BROKER" == 1 ]]; then
  BROKER_BIN="$ROOT/target/debug/broker"
  if [[ ! -x "$BROKER_BIN" ]]; then
    echo "building broker…" >&2
    (cd "$ROOT" && cargo build -p broker)
  fi
  RESTRICT='command="'"$BROKER_BIN"'",no-port-forwarding,no-x11-forwarding,no-agent-forwarding '
else
  RESTRICT=''
fi
printf '%s%s\n' "$RESTRICT" "$(cat "$STATE/client_ed25519.pub")" > "$STATE/authorized_keys"
chmod 600 "$STATE/authorized_keys"

cat > "$STATE/sshd_config" <<EOF
Port $PORT
ListenAddress 127.0.0.1
HostKey $STATE/host_ed25519
PidFile $STATE/sshd.pid
AuthorizedKeysFile $STATE/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
PermitRootLogin no
StrictModes no
AllowUsers $USER
LogLevel VERBOSE
EOF

FP="$(ssh-keygen -lf "$STATE/host_ed25519.pub" | awk '{print $2}')"
echo "loopback sshd starting on 127.0.0.1:$PORT (broker: $USE_BROKER)"
echo "  host key fingerprint : $FP"
echo "  client identity      : $STATE/client_ed25519"
echo "  try: ssh -i $STATE/client_ed25519 -p $PORT -o UserKnownHostsFile=$STATE/known_hosts -o StrictHostKeyChecking=accept-new $USER@127.0.0.1 tmux list-sessions"
exec "$SSHD_BIN" -f "$STATE/sshd_config" -D -e
