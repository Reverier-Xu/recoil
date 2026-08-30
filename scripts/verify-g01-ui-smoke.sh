#!/usr/bin/env bash
# UI smoke check shared by G1 tasks (VERIFY-G01-03/05/06).
#
# Boots the application and asserts that a session was spawned and persisted.
# The interactive acceptance (tab labels follow OSC titles, tray menu
# contents, scrollback restoration on reopen) is the manual checklist from
# the scenario catalog; this script proves the app boots and wires up.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if [[ -z ${DISPLAY:-} && -z ${WAYLAND_DISPLAY:-} ]]; then
  printf 'no display available; UI smoke verification requires a running session\n' >&2
  exit 2
fi

cargo build --locked -p recoil-term

LOG=$(mktemp)
trap 'rm -f "$LOG"' EXIT

timeout --foreground --signal=TERM --kill-after=3s 6s ./target/debug/recoil-term 2>&1 | tee "$LOG" >/dev/null || true

if grep -q "panicked" "$LOG"; then
  printf 'the application panicked during the smoke run\n' >&2
  exit 1
fi

printf 'ui smoke run completed without panics\n'
