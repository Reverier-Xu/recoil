#!/usr/bin/env bash
# Run the registered verification for one task, then the repository quality
# suite when the task requires it. Usage: scripts/verify-task.sh T-Gxx-yy
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

if [[ $# -ne 1 || ! $1 =~ ^T-G[0-9]{2}-[0-9]{2}$ ]]; then
  printf 'usage: scripts/verify-task.sh T-Gxx-yy\n' >&2
  exit 2
fi
TASK_ID=$1

scripts/validate-planning-docs.sh
command -v timeout >/dev/null 2>&1 || { printf 'missing timeout utility\n' >&2; exit 2; }

MANIFEST=$(taplo get -o json -f docs/task-verification.toml)
TASK=$(jq -c --arg id "$TASK_ID" '.task[] | select(.id == $id)' <<<"$MANIFEST")
[[ -n $TASK ]] || { printf 'unknown task: %s\n' "$TASK_ID" >&2; exit 2; }

STATE=$(jq -r '.state' <<<"$TASK")
VERIFY_ID=$(jq -r '.verification_id' <<<"$TASK")
case "$STATE" in
  ready) ;;
  planned)
    printf '%s is planned; register literal argv and change state to ready before starting\n' "$TASK_ID" >&2
    exit 2
    ;;
  *)
    printf 'invalid task state for %s: %s\n' "$TASK_ID" "$STATE" >&2
    exit 2
    ;;
esac

VERIFICATION=$(jq -c --arg id "$VERIFY_ID" '.verification[] | select(.id == $id)' <<<"$MANIFEST")
[[ -n $VERIFICATION ]] || { printf 'missing verification: %s\n' "$VERIFY_ID" >&2; exit 2; }
TIMEOUT_SECONDS=$(jq -r '.timeout_seconds' <<<"$VERIFICATION")
printf 'running %s for %s\n' "$VERIFY_ID" "$TASK_ID"
mapfile -t ARGV < <(jq -r '.argv[]' <<<"$VERIFICATION")
timeout --foreground --signal=TERM --kill-after=30s "${TIMEOUT_SECONDS}s" "${ARGV[@]}"

if [[ $(jq -r '.include_quality' <<<"$TASK") == true ]]; then
  QUALITY_TIMEOUT_SECONDS=$(jq -r '.quality_timeout_seconds' <<<"$MANIFEST")
  while IFS= read -r command; do
    [[ -z $command ]] && continue
    printf 'quality: %s\n' "$command"
    timeout --foreground --signal=TERM --kill-after=30s \
      "${QUALITY_TIMEOUT_SECONDS}s" bash -c "$command"
  done < <(jq -r '.quality_argv[] | join(" ")' <<<"$MANIFEST")
fi

printf '%s verified\n' "$TASK_ID"
