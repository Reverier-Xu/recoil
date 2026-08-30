# Recoil Implementation Task Plan

## Responsibility

This is the executable backlog derived from
[Development Gates](development-gates.md). A task begins only with literal
argv registered in `task-verification.toml`, passes its focused evidence and
`Q`, and changes no contract beyond this plan. Tasks own the paths listed in
[evidence-impact.toml](evidence-impact.toml).

Rollback codes: `R0` independent; `R1` before dependents; `R2` must preserve
compatibility of shipped config/state files; `R3` irreversible action.

## Package Decisions

- `woocraft` (git, pinned rev) is the only GUI dependency entry point; its
  re-exports provide `gpui`, `gpui_macros`, `gpui_sum_tree`, and
  `woocraft-terminal` (with `alacritty_terminal`).
- `recoil-core` stays free of GUI dependencies forever.
- serde + TOML own configuration; JSON owns machine-written state.
- `tracing` owns diagnostics; `rust-i18n` owns user-facing strings.
- No embedded SSH library in this program; the ssh binary is the transport
  (ADR-0002).
- `notify` owns filesystem watching for configuration hot reload
  (T-G02-02).

## Critical Path

`G0 -> G1 -> G2 -> G3` and `G1 -> G4 -> G5`, converging on `G6 -> G7`.
G3 and G4 may run in parallel after their dependencies pass.

## G0: Project Baseline and Contract Freeze

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G00-01 Workspace and crate boundaries | P0/M | — | Virtual workspace, `recoil-core` (headless) and `recoil-term` (bin) crates, pinned `woocraft` through re-exports only, workspace lints | `Cargo.toml`, `crates/*` | `Q` | R0 |
| T-G00-02 Governance documents and ADRs | P0/M | 00-01 | AGENTS.md, DESIGN.md (EN), roadmap, gates, this plan, decision register, scenario catalog, evidence impact, threat model, ADR-0001..0003 | `AGENTS.md`, `docs/**` | Roadmap and gates verify blocks; `scripts/validate-planning-docs.sh` | R0 |
| T-G00-03 Planning validator and task dispatcher | P0/M | 00-02 | `scripts/validate-planning-docs.sh` (schema, status, ID cross-consistency) and `scripts/verify-task.sh` (argv dispatch + quality suite) | `scripts/**` | Validator self-run; negative fixture check | R0 |
| T-G00-04 CI quality workflow | P0/M | 00-03 | `quality_check.yml` with format, planning, msrv, stable, features, dependency-policy jobs; pinned actions | `.github/**` | Pushed run green on `main` and PRs | R0 |
| T-G00-05 Dependency policy | P1/L | 00-01 | `deny.toml`: advisory, license, and ban policy incl. git-source allowlist for `woocraft`/`zed`/`alacritty` | `deny.toml` | `cargo deny --locked --workspace check` | R0 |
| T-G00-06 Config model skeleton | P1/M | 00-01 | `TerminalConfig`/`Config` with px font sizing, `scrolling-history = 0` unlimited sentinel, validation, round-trip and boundary tests | `crates/recoil-core/src/config*` | `Q`; focused config tests | R0 |

## G1: Workbench Shell and Session Ownership

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G01-01 Dock shell and session-state persistence | P0/M | G0 | `Workspace` root with `DockArea`, left dock three-tab skeleton, main tab area, title bar and app menu; `state.json` round-trips the open sessions (fresh local shells in their last local directories) and the active terminal | `crates/recoil-term/src/workspace*` | `Q`; session-state persistence test; SC-G01-P0-01..02 | R1 |
| T-G01-02 Session store and lifecycle FSM | P0/H | 01-01 | `SessionStore` global, `SessionEntry`, `SessionState`, `SessionId` (ULID), ADR-0001 state machine, debounced metadata events | `crates/recoil-term/src/stores/sessions*` | `Q`; headless FSM tests; SC-G01-P0-03..05 | R1 |
| T-G01-03 Terminal panel | P0/H | 01-02 | `TerminalPanel` (Panel impl) spawning sessions from settings, OSC title tab tracking, focus handling, per-view event pump only | `crates/recoil-term/src/terminal/*` | `Q`; SC-G01-P0-06..07 | R1 |
| T-G01-04 Close-path semantics | P0/H | 01-02,01-03 | Tab close detaches, tree close kills (incl. backgrounded), root exit cleanup; headless evidence with real PTYs | `stores/sessions`, `terminal/panel` | Headless PTY lifecycle tests; E2E-01 | R1 |
| T-G01-05 Tray and keybindings | P1/M | 01-03 | Tray feature wiring (show/hide/quit with session confirmation), action registry and default keymap | `tray*`, `actions*` | `Q`; SC-G01-P1-01..02 | R1 |
| T-G01-06 View restoration | P0/M | 01-04 | Reopen a backgrounded session from the store: view rebuild with scrollback, title, and kind metadata | `terminal/panel`, `stores/sessions` | E2E-02 checklist | R1 |

## G2: Configuration System

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G02-01 Full config surface | P0/H | G1 | `[terminal]` (font family/fallbacks/px size, cursor shape/blink, scrollback incl. unlimited), `[theme]` (mode, palette override), `[terminal.features]`; JSON Schema export | `crates/recoil-core/src/config*` | `Q`; round-trip/boundary properties | R2 |
| T-G02-02 Settings store and hot reload | P0/H | 02-01 | `SettingsStore` global with file watching, atomic swap, `SettingsChanged` events, debounced writes | `crates/recoil-term/src/stores/settings*` | Hot reload tests; E2E-03 | R2 |
| T-G02-03 Settings UI | P1/M | 02-02 | Settings window: terminal/appearance/features pages on woocraft Form; validation diagnostics surface | `crates/recoil-term/src/panels/settings*` | SC-G02-P1-01..03 | R1 |
| T-G02-04 Appearance wiring | P0/H | 02-02 | Font family/size/fallbacks, theme mode with terminal palette override, cursor shape/blink applied to live sessions; woocraft upstream hooks landed or shimmed | `terminal/*`, `stores/settings` | E2E-03; SC-G02-P0-04..05 | R1 |
| T-G02-05 Unlimited scrollback | P1/H | 02-01 | `scrolling-history = 0` disk-backed scrollback; upstream `woocraft-terminal` pager designed and registered (implemented here only if upstream lands the hook) | `crates/recoil-core/src/config*`, upstream issue | Specification review; upstream issue link | R2 |

## G3: Terminal Extensions

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G03-01 Hyperlinks | P0/H | G2 | OSC 8 hover underline, tooltip, modifier-click open with scheme allowlist, copy-link menu; upstream link events landed or shimmed | `terminal/*` | SC-G03-P0-01..03 | R1 |
| T-G03-02 Smart selection | P0/H | G2 | Word-character configuration, click/double/triple granularity, block select, lazy URL fallback detection over visible rows | `terminal/*` | SC-G03-P0-04..06 | R1 |
| T-G03-03 OSC 7 cwd tracking | P0/M | G2 | `CwdChanged` event (upstream) or shim; shell integration snippet; feed into activity store | `terminal/*`, `stores/activity*` | SC-G03-P0-07..08 | R1 |
| T-G03-04 Context menu and search | P1/H | 03-01,03-02 | Terminal context menu (copy/paste/select-all/clear/open-link); scrollback search with highlight or documented degraded form | `terminal/*` | SC-G03-P1-01..03 | R1 |
| T-G03-05 OSC 52 and bell | P1/M | 03-04 | Clipboard store/load gated by feature switch; bell → tab highlight and optional background notification | `terminal/*` | SC-G03-P1-04..05 | R1 |

## G4: SSH Profile Management

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G04-01 Profile model and store | P0/M | G1 | `SshProfile` (auth, jump chains, tags, groups), `SshProfileStore` with validation and jump-cycle detection | `crates/recoil-core/src/ssh*`, `stores/ssh*` | `Q`; validation tests | R2 |
| T-G04-02 Ssh transport | P0/H | 04-01 | `SshTransport` trait; ssh binary implementation (jump, key/agent, port, init command, env); argument construction tests without network | `crates/recoil-term/src/ssh/*` | Argument/unit suite; SC-G04-P0-01..03 | R1 |
| T-G04-03 Profile CRUD UI | P1/M | 04-01 | Profile list, form editor, group tree; connection action creating classified sessions | `crates/recoil-term/src/panels/ssh*` | SC-G04-P1-01..03 | R1 |
| T-G04-04 Import and status | P0/M | 04-02 | `~/.ssh/config` import with strict parsing (no shell interpolation); connection status metadata into session store | `ssh/*` | E2E-05; SC-G04-P0-04..05 | R1 |

## G5: Session Management

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G05-01 Activity store and path/history panels | P0/M | G1,03-03 | Shared `ActivityStore` (paths + closed sessions) with caps and LRU; Paths and History panels with search | `stores/activity*`, `panels/paths*`, `panels/history*` | `Q`; projection tests; SC-G05-P0-01..02 | R2 |
| T-G05-02 Sessions panel and classification | P0/H | G4 | Sessions panel with time / `ssh:cwd` / custom-tree views and search; virtualized rendering | `panels/sessions*` | SC-G05-P0-03..05 | R1 |
| T-G05-03 Custom groups | P0/M | 05-02 | Drag-and-drop grouping, auto-group rules by profile/host pattern, group persistence | `panels/sessions*`, `stores/sessions` | SC-G05-P0-06..07 | R2 |
| T-G05-04 State persistence | P0/M | 05-01..03 | `state.json` atomic writes, corruption recovery, restart reopen suggestions | `stores/*` | E2E-06, E2E-07 | R2 |

## G6: Background Residency

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G06-01 Close-to-hide and platform probing | P0/H | G5 | Capability probe (tray, hide), close → hide semantics with degradation chain per platform | `workspace*`, `platform*` | E2E-08 matrix | R1 |
| T-G06-02 Dynamic tray and single instance | P1/M | 06-01 | Tray menu mirrors active sessions; single-instance DBus protocol waking the first instance | `tray*` | SC-G06-P0-01..02 | R1 |
| T-G06-03 Notifications and crash safety | P1/M | 06-01 | Background bell notifications; atomic state writes with corruption fallback exercised | `stores/*` | SC-G06-P0-03..04 | R2 |

## G7: Release Hardening

| Task | Risk | Depends | Deliverable / exact impact | Owned paths | Evidence | RB |
| --- | --- | --- | --- | --- | --- | --- |
| T-G07-01 i18n completeness audit | P0/M | G6 | Every key in `en-US` and `zh-CN`; grep gate for hardcoded user-facing strings | `crates/recoil-term/locales/**` | Audit script output | R0 |
| T-G07-02 Packaging | P1/M | G6 | Desktop entry, icons, release profile, install smoke per platform | `packaging/**` | Install smoke checklist | R0 |
| T-G07-03 Performance audit | P0/M | G6 | Latency and memory measurements against decision-register budgets; soak report | `docs/**`, `scripts/**` | E2E-09 report | R0 |
| T-G07-04 User documentation | P1/L | G6 | README, user manual, keymap reference | `docs/**`, `README.md` | Review | R0 |

## Task Verification

Every task ID here must appear in
[task-verification.toml](task-verification.toml) with a `verification_id`.
Register literal argv and set `state = "ready"` before starting a task; run
`scripts/verify-task.sh T-Gxx-yy` for focused evidence plus `Q`.

```bash
test -f docs/implementation-plan.md
test "$(wc -l < docs/implementation-plan.md)" -le 300
```
