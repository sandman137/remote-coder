#!/usr/bin/env bash
# Tier-1 attention hook for Claude Code (DESIGN.md §9).
#
# Install in ~/.claude/settings.json:
#   {
#     "hooks": {
#       "Notification": [{ "matcher": "", "hooks": [
#         { "type": "command", "command": "/opt/rcoder/claude-code-hook.sh" }
#       ]}]
#     }
#   }
#
# Claude Code invokes this when it needs attention; we forward a
# privacy-filtered payload ({session,pane,state,agent} ONLY — the hook's
# stdin JSON, which may contain message text, is deliberately not read).
#
# Env: NOTIFIER_BIN (default: notifier on PATH), NTFY_URL/NTFY_TOPIC pass
# through to the notifier.
set -u

# Outside tmux there is nothing to point a remote at; exit quietly.
[[ -n "${TMUX_PANE:-}" ]] || exit 0

SESSION="$(tmux display-message -p -t "$TMUX_PANE" '#{session_name}' 2>/dev/null)" || exit 0
NOTIFIER="${NOTIFIER_BIN:-notifier}"

exec "$NOTIFIER" notify \
    --session "$SESSION" \
    --pane "$TMUX_PANE" \
    --state waiting \
    --agent claude-code
