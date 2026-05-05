#!/usr/bin/env bash
# PostToolUse:Edit|Write — runs `mmk review`. Falls back to a one-shot
# advisory if mmk isn't on PATH so the plugin doesn't silently no-op.
set -u
if ! command -v mmk >/dev/null 2>&1; then
  cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"mokumokuren plugin is installed but the `mmk` binary is not on PATH. Install: https://github.com/Mike-Litton/mokumokuren#install — then restart this Claude Code session."}}
JSON
  exit 0
fi
exec mmk review
