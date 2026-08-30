# Recoil Development Gates

## Reason for Existence

This document orders the implementation tasks and their evidence. Gates map
one-to-one onto roadmap milestones M0–M7 and to the phase plan in
[DESIGN.md](DESIGN.md).

## Gate Protocol

A gate is `NOT_STARTED`, `ACTIVE`, `BLOCKED`, or `PASS`.

- A gate passes only from automated evidence for its current predicates.
- Work enters a gate only after dependencies pass. Existing code is
  re-evaluated instead of presumed complete.
- Required failures, skips, or retries-until-green block closure.
- `Q` is the repository quality suite documented in `AGENTS.md` and
  `task-verification.toml`.

## Verification Architecture

| Level | Required evidence |
| --- | --- |
| Unit | Config validation, classification projections, lifecycle FSM transitions cover valid/boundary/invalid paths (`recoil-core`) |
| Headless integration | Real `TerminalSession` PTY round trips, event ordering, close-kill semantics without a window |
| Scenario | UI acceptance scenarios from `scenario-catalog.toml`, exercised manually with a recorded checklist per gate |
| E2E | The E2E catalog below, automated where a window is not required, checklist-driven where it is |
| Platform | Linux (X11, Wayland), macOS, Windows behavior per the G6 matrix with zero warnings |
| i18n | Every key present in `en-US` and `zh-CN`; no hardcoded user-facing strings (grep gate) |
| Performance | Input latency and memory budgets from `decision-register.toml`, measured at G7 |

## Development Gates

### G0: Project Baseline and Contract Freeze

- **Build:** Workspace with `recoil-core` and `recoil-term`; `woocraft` git
  dependency pinned through re-exports; config model skeleton with
  validation; AGENTS.md, roadmap, gates, plan, manifests, ADR-0001..0003;
  planning validator; verify-task dispatcher; CI workflow.
- **Depends on:** Accepted ADR-0001..0003 and this document set.
- **Verify:** `Q`; `scripts/validate-planning-docs.sh`; CI green on push.
- **Pass:** The contract and module boundaries are frozen; all later tasks
  reference stable `T-Gxx-yy` IDs.

### G1: Workbench Shell and Session Ownership

- **Build:** DockArea assembly with left dock (three tabs) and main tab
  area; session-state persistence (open sessions and the active terminal
  round-trip through `state.json` as fresh local shells); `SessionStore` with `SessionEntry`,
  `SessionState`, and the ADR-0001 state machine; `TerminalPanel` with OSC
  title tracking; close-path semantics (tab close detaches, tree close
  kills, root exit cleans up); basic tray with show/hide/quit; view
  restoration from a live session; core keybindings.
- **Depends on:** G0.
- **Verify:** `Q`; headless session FSM tests; E2E-01, E2E-02.
- **Pass:** Session ownership exists only in the session store; the three
  close paths behave exactly as ADR-0001 specifies.

### G2: Configuration System

- **Build:** Full config surface (terminal, theme, features) with
  validation and defaults from `decision-register.toml`; file watching with
  hot reload; JSON Schema export; settings UI (terminal/appearance/features);
  font family/px size/fallbacks wiring; theme mode and terminal palette
  override; cursor shape/blink; per-feature switches; unlimited
  disk-backed scrollback specification (`scrolling-history = 0`) with its
  upstream dependency registered or implemented.
- **Depends on:** G1.
- **Verify:** `Q`; config property/round-trip tests; hot reload E2E-03.
- **Pass:** Every product-contract setting is editable, hot-applied, and
  persisted; invalid configurations are rejected with diagnostics.

### G3: Terminal Extensions

- **Build:** OSC 8 hyperlink hover/open/copy with modifier requirement;
  smart selection (word characters, multi-click granularity, block select,
  URL fallback); OSC 7 cwd tracking into the activity store with the shell
  integration snippet; terminal context menu; scrollback search; OSC 52 and
  bell gating; upstream hooks listed in `DESIGN.md` landed or shimmed with
  issue links.
- **Depends on:** G2.
- **Verify:** `Q`; extension scenario sets; E2E-04.
- **Pass:** Every extension is functional and fully disabled when its
  feature switch is off; no extension adds work to the hot path when idle.

### G4: SSH Profile Management

- **Build:** `SshProfile` model with auth/jump/tags/groups; profile store;
  CRUD UI with form and group tree; `SshTransport` trait plus ssh binary
  implementation; `~/.ssh/config` import with strict parsing; connection
  status metadata feeding the session store.
- **Depends on:** G1.
- **Verify:** `Q`; transport argument-construction tests (no real network in
  unit evidence); E2E-05.
- **Pass:** A profile connects with agent and key auth; imported profiles
  round-trip; no secret reaches disk, logs, or tracing output.

### G5: Session Management

- **Build:** Paths/history/sessions panels with search; time, `ssh:cwd`,
  and custom-tree classification views; drag-and-drop grouping with
  auto-group rules; activity store with caps and LRU shrink; `state.json`
  atomic persistence; restart-time reopen suggestions.
- **Depends on:** G1 and G4.
- **Verify:** `Q`; projection unit tests; E2E-06, E2E-07.
- **Pass:** 50+ sessions remain fluid; all three views and search agree with
  the store; custom groups survive restart.

### G6: Background Residency

- **Build:** Close-to-hide with platform probing and degradation chain;
  dynamic tray session menu; single-instance protocol; background bell
  notifications; crash-safe state writes with corruption recovery.
- **Depends on:** G5 (and G2 for feature switches).
- **Verify:** `Q`; platform matrix checklist; E2E-08.
- **Pass:** Every supported platform hides/restores correctly or degrades
  explicitly; second launches wake the first instance.

### G7: Release Hardening

- **Build:** Tier-0 locale completeness audit; packaging (desktop entry,
  icons, release profile); performance audit (long scrollback, 100-session
  soak, latency measurement); user documentation (README, manual, keymap).
- **Depends on:** G6 and all prior gates.
- **Verify:** `Q`; i18n grep gate; E2E-09; packaging install smoke on each
  platform.
- **Pass:** No P0/P1 findings; budgets from `decision-register.toml` hold;
  documentation complete.

## Required E2E Catalog

| ID | Scenario | Owner gate | Acceptance |
| --- | --- | --- | --- |
| E2E-01 | Close-semantics matrix | G1 | Tab close keeps the PTY alive; tree close kills it including backgrounded sessions; root exit removes entry and tab; all transitions idempotent |
| E2E-02 | Residency round trip | G1 | Hide window and reopen: sessions survive, restored views show full scrollback and titles |
| E2E-03 | Config hot reload | G2 | Saving `config.toml` applies font, theme, cursor, and feature switches to live sessions without restart; invalid file is rejected with diagnostics |
| E2E-04 | Terminal extension flow | G3 | URL opens with modifier-click; word/line/block selection honors word characters; `cd` updates the path store; all behaviors follow their switches |
| E2E-05 | SSH profile lifecycle | G4 | Import, group, connect over agent auth, disconnect; session classified as `ssh:host`; no secret on disk or in logs |
| E2E-06 | Panel scale | G5 | 50+ sessions across all three views with search stay interactive under store-event debouncing |
| E2E-07 | Cross-restart recovery | G5 | Custom groups, history, and layout survive restart; dead sessions are offered for reopen, not resurrected |
| E2E-08 | Platform residency matrix | G6 | Hide-to-tray, tray menu, and single instance behave per matrix on Linux X11/Wayland, macOS, Windows, with explicit degradation where unsupported |
| E2E-09 | Performance soak | G7 | 100 concurrent sessions and 1,000,000-line unlimited scrollback hold latency and memory budgets |

## Handoff to Implementation

[Implementation Plan](implementation-plan.md) owns the task rows. A task
handoff uses the current row and scenarios, never superseded evidence.

```bash
test -f docs/development-gates.md
test "$(wc -l < docs/development-gates.md)" -le 300
test "$(grep -c '^### G[0-9]' docs/development-gates.md)" -eq 8
test "$(grep -c '^| E2E-' docs/development-gates.md)" -eq 9
```
