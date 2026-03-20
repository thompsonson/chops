#!/bin/bash
# Wrapper for ttyd: attach to the tmux session named in the state file.
# Falls back to the first available session if the file is missing or
# the named session doesn't exist.

STATE_FILE="${CHOPS_TTYD_STATE:-$HOME/.config/chops/ttyd-session}"
TMUX_TMPDIR="${TMUX_TMPDIR:-/tmp}"
export TMUX_TMPDIR

# Read desired session from state file
session=""
if [ -f "$STATE_FILE" ]; then
  session=$(cat "$STATE_FILE" 2>/dev/null | tr -d '[:space:]')
fi

# Validate session exists
if [ -n "$session" ] && tmux has-session -t "=$session" 2>/dev/null; then
  exec tmux attach -t "=$session"
fi

# Fallback: first available session
first=$(tmux list-sessions -F '#{session_name}' 2>/dev/null | head -1)
if [ -n "$first" ]; then
  exec tmux attach -t "=$first"
fi

echo "No tmux sessions available."
sleep 5
