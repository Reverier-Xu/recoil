#!/usr/bin/env bash
# VERIFY-G01-01: dock shell assembly and layout persistence.
#
# Boots the application for a few seconds, then asserts that the persisted
# workspace state contains the left dock panels and a terminal panel.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if [[ -z ${DISPLAY:-} && -z ${WAYLAND_DISPLAY:-} ]]; then
  printf 'no display available; UI smoke verification requires a running session\n' >&2
  exit 2
fi

cargo build --locked -p recoil-term

STATE_FILE=
if [[ -n ${XDG_CONFIG_HOME:-} ]]; then
  STATE_FILE="$XDG_CONFIG_HOME/recoil/state.json"
elif [[ -n ${HOME:-} ]]; then
  STATE_FILE="$HOME/.config/recoil/state.json"
fi
[[ -n $STATE_FILE ]] || { printf 'cannot determine the state file location\n' >&2; exit 2; }
rm -f "$STATE_FILE"

timeout --foreground --signal=TERM --kill-after=3s 6s ./target/debug/recoil-term >/dev/null 2>&1 || true
sleep 1

[[ -f $STATE_FILE ]] || { printf 'state file was not written: %s\n' "$STATE_FILE" >&2; exit 1; }

for needle in '"PathsPanel"' '"HistoryPanel"' '"SessionsPanel"' '"TerminalPanel"'; do
  grep -q "$needle" "$STATE_FILE" || {
    printf 'persisted layout is missing %s\n' "$needle" >&2
    exit 1
  }
done

printf 'layout persistence verified (%s)\n' "$STATE_FILE"
