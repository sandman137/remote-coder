#!/usr/bin/env bash
# Spin up (or refresh) the local "agents" tmux session with fake agents,
# one per window. This is the dev fixture the TUI harness and integration
# flows point at (DESIGN.md §11 `just fake-session`).
#
# Env overrides:
#   HELM_SESSION      session name           (default: agents)
#   HELM_TMUX_SOCKET  tmux -L socket name    (default: default server)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SESSION="${HELM_SESSION:-agents}"

TMUX=(tmux)
if [[ -n "${HELM_TMUX_SOCKET:-}" ]]; then
  TMUX=(tmux -L "$HELM_TMUX_SOCKET")
fi

if "${TMUX[@]}" has-session -t "=$SESSION" 2>/dev/null; then
  "${TMUX[@]}" kill-session -t "=$SESSION"
fi

# A generous size so agent TUIs render like they would on a desktop;
# clients reflow it to their own viewport (window-size latest).
"${TMUX[@]}" new-session -d -s "$SESSION" -n yn -x 100 -y 30 \
  "$ROOT/fixtures/agents/fake-yn.sh"
"${TMUX[@]}" set-option -t "=$SESSION" -g window-size latest
"${TMUX[@]}" new-window -t "=$SESSION" -n numbered "$ROOT/fixtures/agents/fake-numbered.sh"
"${TMUX[@]}" new-window -t "=$SESSION" -n stream "$ROOT/fixtures/agents/fake-stream.sh"
"${TMUX[@]}" select-window -t "=$SESSION:yn"

echo "tmux session '$SESSION' ready:"
"${TMUX[@]}" list-panes -s -t "=$SESSION" \
  -F '#{session_name}:#{window_index}.#{pane_index}  #{window_name}  #{pane_current_command}  #{pane_width}x#{pane_height}'
