#!/usr/bin/env bash
# PostToolUse:Bash(git commit:*) — runs `mmk session-summary` against the
# upstream default branch when discoverable, falling back to `main`.
set -u
if ! command -v mmk >/dev/null 2>&1; then
  exit 0  # missing-binary message already surfaced by review-on-edit.sh
fi
base="$(git symbolic-ref --quiet refs/remotes/origin/HEAD 2>/dev/null | sed 's@.*/@@')"
exec mmk session-summary --base "${base:-main}" --drift-sessions 5
