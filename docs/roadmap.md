# Recoil Development Roadmap

## Reason for Existence

This document turns the accepted product boundary into an ordered, verifiable
development program. ADRs govern design choices;
[ADR-0001](adr/0001-session-ownership-and-lifecycle.md) is authoritative for
session lifecycle semantics.

`recoil` is an IDE-style terminal emulator. It is terminal-first and
performance-first: the terminal grid is the primary surface, and all
peripheral UI (docks, panels, configuration) serves the terminal experience
without ever adding cost to the terminal hot path.

## Product Contract

The application provides:

- A GPU-rendered terminal surface (`woocraft::TerminalView`) with mouse
  reporting, multi-granularity selection, bracketed paste, OSC 52 clipboard
  integration, and application titles.
- Terminal extensions as switchable features: OSC 8 hyperlinks with
  modifier-click opening, smart selection driven by configurable word
  characters, scrollback search, OSC 7 working-directory tracking, and bell
  notifications.
- Session ownership independent of views: terminal sessions survive tab
  close, window close, and dock removal, per the lifecycle state machine in
  ADR-0001.
- Dynamic session observations: the working directory and ssh state follow
  what the user does inside the terminal (`cd`, `ssh`, exit); the
  application derives them from terminal behavior without intruding on user
  operations, and classification views always reflect the current
  observation.
- Configuration management: a validated TOML configuration (terminal font
  family with fallbacks and px sizing, cursor shape and blink, theme mode
  with terminal palette overrides, per-feature switches) with hot reload and
  a settings UI.
- SSH connection management: editable profiles with host/port/user/auth,
  jump chains, grouping, tags, and `~/.ssh/config` import, connected through
  the ssh binary transport (ADR-0002).
- Session management: a registry of active sessions classified by time, by
  `ssh:cwd`, and by user-defined trees, with search; plus a shared activity
  store powering the paths and history panels.
- Background residency: tray presence, hide-to-tray window semantics, and a
  single-instance protocol.
- Scrollback: finite in-memory history (default 10,000 lines, max
  1,000,000), or `scrolling-history = 0` meaning unlimited with a
  disk-backed cache in the konsole/kitty style.
- Full i18n with tier-0 locales `en-US` and `zh-CN`.
- Portable behavior across Linux (X11 and Wayland where the platform allows),
  macOS, and Windows, with graceful degradation where a platform lacks a
  capability (tray, hide-to-tray).

## Non-Goals

- Embedded SSH transport in this program: no libssh2/russh connection pool,
  SFTP browser, or port-forward UI. The `SshTransport` trait reserves the
  extension point; implementing it is out of scope until a concrete need is
  accepted by ADR.
- Shell integration beyond OSC 7 and opt-in prompt markers: no proprietary
  shell framework, no command statistics beyond the history panel.
- Remote sessions other than the ssh binary wrapper (mosh, telnet, serial,
  WSL) — reserved behind the same transport trait.
- Plugin systems, custom widget scripting, or theming beyond token/palette
  overrides.
- Terminal multiplexing features (persistent server sessions beyond the
  application's own process lifetime).

## Architecture Rules

1. The terminal hot path (PTY read → emulator → grid paint) never allocates
   locks, channels, or notifications for peripheral subsystems. Terminal
   `Wakeup` events never invalidate stores or dock panels.
2. PTY sessions are owned only by the session store. Views, tabs, dock
   panels, and windows are observers created on demand.
3. Stores are the single source of truth; panels are projections and hold no
   authoritative state except transient UI state (expanded nodes, scroll
   positions).
4. Low-frequency session events (title, cwd, exit) reach stores debounced;
   high-frequency content updates never do.
5. Everything user-visible is an i18n key; adding a key without both tier-0
   translations fails review.
6. Every feature switch documented in the product contract must be observable
   in configuration and have a default that keeps the feature enabled unless
   safety says otherwise.
7. Platform capabilities (tray, hide-to-tray, single instance) are probed at
   startup and degrade explicitly; behavior differences are documented in the
   platform matrix owned by G6.
8. Forbid `unsafe`, production `unwrap()`, and production `expect()`.
   Diagnostics exclude secrets, credentials, and unredacted user paths.
9. All GUI dependencies enter through `woocraft` re-exports at the pinned
   revision. Capability gaps are fixed upstream first (see the upstream
   collaboration register in `docs/DESIGN.md`).

## Planned Module Boundaries

| Area | Crate | Responsibility | Primary extension boundary |
| --- | --- | --- | --- |
| Domain model | `recoil-core` | Config, profiles, session metadata, classification, persistence | Serde data model |
| Workspace shell | `recoil-term` | Dock assembly, session-state persistence, actions, keybindings | Panel registry |
| Terminal surface | `recoil-term::terminal` | Terminal panels, extensions glue, search, context menu | Upstream `TerminalView` |
| Session ownership | `recoil-term::stores` | Session store, lifecycle state machine, tray glue | Store events |
| Configuration | `recoil-term::stores` | Settings store, hot reload, settings UI | Config schema |
| SSH | `recoil-term::ssh` | Profiles, transport, import | `SshTransport` trait |
| Panels | `recoil-term::panels` | Paths/history/sessions panels, classification views | Store projections |

## Milestones

### M0: Project Baseline and Contract Freeze

Establish the workspace, the governance system (this document set, ADRs,
scripts, CI), and the quality gates. Freeze the product contract and the
module boundaries above.

Exit gate:

- Workspace builds with zero warnings through the full quality suite.
- `scripts/validate-planning-docs.sh` passes; CI executes format, planning,
  msrv, stable, features, and dependency-policy jobs.
- ADR-0001 through ADR-0003 are accepted; planning manifests are consistent.

### M1: Workbench Shell and Session Ownership

Deliver the dock shell (left dock with three tabs, main tab area, title bar,
menu), the session store with the full lifecycle state machine, terminal
panels with OSC title tracking, basic tray (show/hide/quit), and view
restoration from a live session.

Exit gate:

- E2E-01 close-semantics matrix and E2E-02 residency round trip pass.
- Session ownership lives only in the session store; no view holds a PTY.

### M2: Configuration System

Deliver the full config surface (terminal, theme, features), validation,
hot reload, schema export, and the settings UI; wire fonts (family, px size,
fallbacks), theme mode with terminal palette override, cursor shape/blink,
and per-feature switches into every spawn and render path.

Exit gate:

- E2E-03 config hot reload passes; invalid configs are rejected with
  actionable diagnostics.
- Disk-backed unlimited scrollback (`scrolling-history = 0`) is specified and
  either implemented behind its upstream dependency or explicitly tracked
  there with a registered task.

### M3: Terminal Extensions

Deliver hyperlink hover/click/open, smart selection with configurable word
characters and URL fallback, OSC 7 cwd tracking into the activity store,
context menu, scrollback search, OSC 52 gating, and bell handling.

Exit gate:

- E2E-04 terminal extension flow passes; every extension is toggleable and
  disabled state is complete.

### M4: SSH Profile Management

Deliver the profile model, profile store, CRUD UI with group tree, ssh
binary transport (jump chains, keys, agent, ports, init commands),
`~/.ssh/config` import, and connection status metadata.

Exit gate:

- E2E-05 profile connect and classification passes; import never
  interpolates shell text.

### M5: Session Management

Deliver the three left-dock panels with search, the three classification
views, drag-and-drop custom groups with auto-grouping rules, full
`state.json` persistence, and restart-time reopen suggestions.

Exit gate:

- E2E-06 panel scale (50+ sessions) and E2E-07 cross-restart recovery pass.

### M6: Background Residency

Deliver close-to-hide semantics with platform capability probing, dynamic
tray menus, single-instance protocol, background notifications, and
crash-safe atomic state writes.

Exit gate:

- E2E-08 platform residency matrix passes on every supported platform
  configuration.

### M7: Release Hardening

Deliver completed tier-0 locales, packaging (desktop entry, icons, release
profile), performance audits (long scrollback, many sessions), user
documentation, and the platform matrix.

Exit gate:

- E2E-09 performance soak passes; packaging installs and launches on Linux,
  macOS, and Windows; no P0/P1 findings open.

## Requirement Traceability

| Requirement | Owning milestones |
| --- | --- |
| Terminal-first surface with extensions | M1, M3 |
| Session ownership and lifecycle semantics | M1 |
| Configuration management and hot reload | M2 |
| Unlimited disk-backed scrollback | M2 (with upstream dependency) |
| SSH profile management | M4 |
| Session classification, search, custom groups | M5 |
| Background residency and tray | M1, M6 |
| i18n tier-0 completeness | M0 (policy), M2–M7 (per surface), M7 (audit) |
| Portability and platform matrix | M6, M7 |
| Performance-first discipline | M1–M5 (rules), M7 (evidence) |

## Delivery Order

Execution follows [Development Gates](development-gates.md). The critical path
is `G0 -> G1 -> G2 -> G3` and `G1 -> G4 -> G5`, converging on `G6 -> G7`.
G3 and G4 may proceed in parallel after G2 and G1 respectively.

## Definition of Done

Every milestone passes the repository quality suite plus its owned scenarios
and E2E evidence. Review final diffs for accidental production, dependency,
CI, or test changes. Every new UI string exists in both tier-0 locales.

Verify this roadmap with:

```bash
test -f docs/roadmap.md
test "$(wc -l < docs/roadmap.md)" -le 300
grep -q '^## Requirement Traceability$' docs/roadmap.md
grep -q '^### M7: Release Hardening$' docs/roadmap.md
```
