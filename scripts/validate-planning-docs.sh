#!/usr/bin/env bash
# Structural validation of the recoil planning system.
#
# Checks schema/status headers, ID uniqueness, and cross-references between
# the implementation plan, task registry, scenario catalog, threat model,
# decision register, and evidence impact. Run with --self-test to verify the
# validator catches a broken fixture.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

fail() {
  printf 'planning validation failed: %s\n' "$*" >&2
  exit 1
}

SELF_TEST=0
if [[ ${1:-} == "--self-test" ]]; then
  SELF_TEST=1
elif [[ $# -ne 0 ]]; then
  printf 'usage: scripts/validate-planning-docs.sh [--self-test]\n' >&2
  exit 2
fi

for tool in taplo jq awk; do
  command -v "$tool" >/dev/null 2>&1 || fail "missing tool: $tool"
done

DOCS=docs
MANIFEST_NAMES=(task-verification scenario-catalog threat-model decision-register evidence-impact)
declare -A SCHEMA=(
  ["task-verification"]="recoil.woooo.tech/schemas/task-verification-v1"
  ["scenario-catalog"]="recoil.woooo.tech/schemas/planning-scenarios-v1"
  ["threat-model"]="recoil.woooo.tech/schemas/threat-model-v1"
  ["decision-register"]="recoil.woooo.tech/schemas/decision-register-v1"
  ["evidence-impact"]="recoil.woooo.tech/schemas/evidence-impact-v1"
)

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if (( SELF_TEST )); then
  cp -r "$DOCS" "$TMP/docs"
  DOCS="$TMP/docs"
  cat >> "$DOCS/scenario-catalog.toml" <<'FIXTURE'

[[scenario]]
id = "SC-G99-P0-99"
owner_task = "T-G99-99"
gate = "G9"
priority = "P0"
verification_id = "VERIFY-G99-99"
kind = "headless"
title = "broken fixture"
acceptance = "must be rejected because the owner task does not exist."
FIXTURE
fi

for name in "${MANIFEST_NAMES[@]}"; do
  file="$DOCS/$name.toml"
  [[ -f $file ]] || fail "missing manifest: $file"
  taplo lint --no-auto-config "$file" >/dev/null 2>&1 \
    || taplo lint "$file" >/dev/null \
    || fail "taplo lint: $file"
  taplo get -o json -f "$file" > "$TMP/$name.json"
  jq -e --arg schema "${SCHEMA[$name]}" \
    '.schema == $schema and .status == "accepted"' "$TMP/$name.json" >/dev/null \
    || fail "invalid schema/status: $file"
done

# Task IDs in the implementation plan must be unique and well-formed.
mapfile -t PLAN_TASKS < <(grep -oE '^\| T-G[0-9]{2}-[0-9]{2} ' "$DOCS/implementation-plan.md" \
  | awk '{print $2}' | sort)
[[ ${#PLAN_TASKS[@]} -gt 0 ]] || fail "no tasks found in implementation plan"
if [[ $(printf '%s\n' "${PLAN_TASKS[@]}" | uniq -d | wc -l) -ne 0 ]]; then
  fail "duplicate task id in implementation plan"
fi
printf '%s\n' "${PLAN_TASKS[@]}" > "$TMP/plan-tasks.txt"

# Registry tasks must match the plan exactly.
jq -r '.task[].id' "$TMP/task-verification.json" | sort > "$TMP/registry-tasks.txt"
diff -u "$TMP/plan-tasks.txt" "$TMP/registry-tasks.txt" >/dev/null \
  || fail "task registry and implementation plan disagree (see diff above)"

# Every registry task needs a verification entry owned by it, and planned
# tasks must not carry argv yet.
while IFS=$'\t' read -r id vid state; do
  [[ $vid == "VERIFY-${id#T-}" ]] || fail "verification id mismatch for $id"
  row=$(jq -c --arg id "$vid" \
    '.verification[] | select(.id == $id)' "$TMP/task-verification.json")
  [[ -n $row ]] || fail "missing verification $vid for $id"
  [[ $(jq -r '.owner_task' <<<"$row") == "$id" ]] \
    || fail "verification $vid owned by wrong task"
  if [[ $state == "ready" ]]; then
    [[ $(jq -r '.argv | length' <<<"$row") -gt 0 ]] \
      || fail "ready task $id has no verification argv"
  fi
done < <(jq -r '.task[] | [.id, .verification_id, .state] | @tsv' \
  "$TMP/task-verification.json")

# Scenario ids must be unique, well-formed, and reference known tasks and
# verifications.
jq -r '.scenario[] | [.id, .owner_task, .verification_id] | @tsv' \
  "$TMP/scenario-catalog.json" | sort > "$TMP/scenarios.tsv"
[[ $(cut -f1 "$TMP/scenarios.tsv" | uniq -d | wc -l) -eq 0 ]] \
  || fail "duplicate scenario id"
while IFS=$'\t' read -r sid task vid; do
  [[ $sid =~ ^SC-G[0-9]{2}-(P0|P1)-[0-9]{2}$ ]] \
    || fail "malformed scenario id: $sid"
  grep -qx "$task" "$TMP/plan-tasks.txt" \
    || fail "scenario $sid references unknown task $task"
  jq -e --arg id "$vid" '.verification[] | select(.id == $id)' \
    "$TMP/task-verification.json" >/dev/null \
    || fail "scenario $sid references unknown verification $vid"
done < "$TMP/scenarios.tsv"

# Threats reference known scenarios and tasks.
while IFS=$'\t' read -r tid task scenarios; do
  grep -qx "$task" "$TMP/plan-tasks.txt" \
    || fail "threat $tid references unknown task $task"
  if [[ -n $scenarios ]]; then
    while IFS= read -r sid; do
      [[ -z $sid ]] && continue
      grep -q "^$sid" "$TMP/scenarios.tsv" \
        || fail "threat $tid references unknown scenario $sid"
    done < <(jq -Rr 'split(",") | .[]' <<<"$scenarios")
  fi
done < <(jq -r '.threat[] | [.id, .owner_task, ((.scenario_ids // []) | join(","))] | @tsv' \
  "$TMP/threat-model.json")

# Decision register and evidence impact reference known tasks.
for name in decision-register evidence-impact; do
  jq -r '.. | objects | .owner_task? // empty' "$TMP/$name.json" \
    | while IFS= read -r task; do
        grep -qx "$task" "$TMP/plan-tasks.txt" \
          || fail "$name references unknown task $task"
      done
done

if (( SELF_TEST )); then
  printf 'self-test unexpectedly passed with a broken fixture\n' >&2
  exit 1
fi

printf 'planning documents are consistent (%s tasks, %s scenarios)\n' \
  "${#PLAN_TASKS[@]}" "$(wc -l < "$TMP/scenarios.tsv" | tr -d ' ')"
