# Recoil Design Document

IDE-style terminal emulator built on `woocraft` (GPUI). Terminal-first,
performance-first.

> This document is the terminal-state architecture reference. Program
> ordering, gates, and task acceptance live in [roadmap.md](roadmap.md) and
> [development-gates.md](development-gates.md); binding lifecycle decisions
> live in the ADRs.

---

## 1. Position and Design Principles

**Recoil** is a terminal that behaves like an IDE:

- **Terminal first.** The terminal grid is the primary surface. Peripheral
  UI (docks, panels, configuration) serves the terminal experience and never
  adds cost to the terminal hot path.
- **Performance first.** No extra abstraction on the render/input path;
  peripheral state updates are isolated from terminal view updates (precise
  notify, never global invalidation).
- **Sessions are assets.** Terminal sessions are owned by the session
  registry and exist independently of views, windows, and dock layout
  (ADR-0001).
- **Configuration is data.** All configuration is serializable pure data;
  UI is only an editor.
- **Grow upstream.** Generic capabilities (links, smart selection, font
  fallbacks, OSC 7) land in woocraft first; recoil stays glue and product
  semantics.

## 2. Overall Architecture

### 2.1 Crate Layout

```text
recoil/
├── Cargo.toml               # virtual workspace
└── crates/
    ├── recoil-core/         # headless: config, session metadata, classification,
    │                        #   persistence. No gpui dependency; unit-testable.
    └── recoil-term/         # gpui application (bin): stores, panels, views,
                             #   actions, tray glue, locales.
```

Both crates depend on exactly one GUI entry point: the pinned `woocraft` git
dependency, whose re-exports provide `gpui`, `gpui_macros`, `gpui_sum_tree`,
and `woocraft-terminal` (with `alacritty_terminal`). Direct git dependencies
on gpui/zed/alacritty are forbidden.

### 2.2 Runtime Structure

```text
App
├── Globals (gpui Global + Entity singletons)
│   ├── SettingsStore      # recoil-core Config; file watching, hot reload, events
│   ├── SshProfileStore    # profile CRUD, group tree, ~/.ssh/config import
│   ├── SessionStore       # active terminal session registry (sole PTY owner)
│   ├── ActivityStore      # shared paths + history store (paths/history tabs)
│   └── TrayService        # tray lifecycle, menus, event forwarding
├── Workspace (root view)
│   ├── TitleBar + AppMenuBar
│   └── DockArea
│       ├── LeftDock  → TabPanel(PathsPanel | HistoryPanel | SessionsPanel)
│       └── MainArea  → TabPanel(TerminalPanel*)
└── Windows  # multiple windows supported; sessions are store-owned
```

Data flow is strictly one-directional:

```text
PTY I/O thread
  → TerminalSession (async_channel events)
    → TerminalView event pump (4 ms batching)   [hot path: one view only]
    → SessionStore lifecycle subscription        [cold path: title/cwd/exit,
      → ActivityStore (path/history persistence)  debounced per decision register]
      → left dock panels (store subscriptions, virtualized lists)
      → TrayService (menu rebuild)
```

Hard constraint: **terminal `Wakeup` events never trigger store or panel
notifications.** Only low-frequency events enter stores, always debounced.

### 2.3 Session Ownership and Lifecycle

ADR-0001 is authoritative. Summary:

- `SessionStore` owns every `TerminalSession` handle; views are observers
  created on demand from a `SessionId`.
- States: `Spawning → Active ⇄ Backgrounded → Exited → Reaped`.
- Close paths: tab close detaches (PTY untouched); dock-tree close kills
  (the only kill affordance for backgrounded sessions); root exit cleans up
  idempotently; window close backgrounds everything and the tray stays
  resident.
- View restoration rebuilds a `TerminalView` from the stored handle;
  scrollback and emulator state live in the session.

### 2.4 Persistence

```text
~/.config/recoil/
├── config.toml         # terminal + theme + features (hot-reloaded)
├── ssh-profiles.toml   # SSH profiles and groups
└── state.json          # layout, session metadata, custom groups, activity
```

- `config.toml` / `ssh-profiles.toml` are human-first with JSON Schema
  export; load-time validation fails closed with actionable diagnostics.
- `state.json` is machine-written: debounced async persistence with atomic
  tmp+rename writes and corruption recovery (THR-006).
- Custom groups persist in `state.json` keyed by `SessionId`; group
  structure survives member session death.

## 3. Configuration System

```toml
# config.toml
[terminal]
font_family = "Maple Mono"
font_fallbacks = ["Noto Sans Mono CJK SC", "Symbols Nerd Font"]  # upstream hook
font_size = 16.0              # css pixels, matches the woocraft theme default
scrolling_history = 10_000    # lines; 0 = unlimited (disk-backed)
cursor_shape = "block"        # block | underline | bar | hollow
cursor_blink = true
alternate_scroll = true

[terminal.features]           # each switchable
hyperlink = true              # hover underline + modifier-click open
smart_select = true           # word characters + URL fallback
mouse_reporting = true        # application mouse-mode passthrough
copy_on_select = true
osc52 = true                  # allow programs to read/write the clipboard
bell = true
bell_when_hidden_notify = true

[theme]
mode = "system"               # light | dark | system
# optional full 16-color + fg/bg/cursor override; defaults derive from the
# woocraft theme
# [theme.terminal_palette]

[ui]
left_dock_width = 280.0
show_title_bar = true
```

- Font sizes are **css pixels**, never points; the default 16 px matches the
  woocraft theme (`font_size: px(16.)`).
- `scrolling_history = 0` is the unlimited sentinel: scrollback is paged to
  a disk-backed cache in the konsole/kitty style. Finite values are clamped
  to `woocraft-terminal`'s in-memory maximum (1,000,000 lines).
- `SshProfile` (ssh-profiles.toml): `name / host / port / user /
  auth(agent|password|key{path}) / jump / group_path / tags /
  init_command / env / working_dir`. Secrets never persist to disk
  (ADR-0002, THR-004).
- Hot reload: `notify` watches files → validate → atomic swap → emit
  `SettingsChanged`; subscribers (terminal fonts/cursor, theme) react
  precisely.

## 4. Terminal Extensions

| Capability | Design |
| --- | --- |
| Hyperlinks | OSC 8 hyperlinks already exist in the cell model (underlined upstream). Add hover hit-testing, hover affordance, modifier-click open behind a scheme allowlist, and copy-link. Needs an upstream `TerminalView` link event or mouse hook. |
| Smart selection | Word selection via the emulator's semantic escape characters (configurable), multi-click granularity (word/line/block), lazy URL regex fallback over visible rows only. Needs upstream exposure of `semantic_escape_chars` and selection granularity. |
| OSC 7 cwd | Capture in `woocraft-terminal`'s vte layer → new `TerminalEvent::CwdChanged(PathBuf)`; ship an opt-in shell-integration snippet. cwd feeds the activity store. |
| Unlimited scrollback | `scrolling-history = 0` pages scrollback to disk (konsole/kitty style). Requires an upstream scrollback pager in `woocraft-terminal`; specified in T-G02-05, implemented locally only if the upstream hook lands. |
| Scrollback search | Incremental regex search over session snapshots with highlight-and-jump; needs an upstream match-highlight hook, else a documented degraded form (jump + select). |
| OSC 52 | `ClipboardStore/Load` events exist; the host honors them only when the feature switch is on (THR-001). |
| Bell | Bell event → tab highlight when visible, system notification when hidden. |

Every internal change to `TerminalElement`/`TerminalView` is proposed
upstream first; recoil-side shims carry upstream issue links and are deleted
when the upstream lands.

### Upstream Collaboration Register

| # | Need | Gate |
| --- | --- | --- |
| 1 | `TerminalView` link hover/click events (or a mouse hook) | G3 |
| 2 | Smart selection: configurable `semantic_escape_chars` + granularity API | G3 |
| 3 | OSC 7 → `TerminalEvent::CwdChanged` | G3 |
| 4 | Terminal font fallback list in the shaping path | G2 |
| 5 | View-restore initial snapshot confirmation / restore API | G1 |
| 6 | Scrollback search highlight hook | G3 (degradable) |
| 7 | Runtime cursor shape/blink setters on `TerminalView` | G2 |
| 8 | Disk-backed scrollback pager for unlimited history | G2 |

## 5. Left Dock and Panels

The left dock is a `TabPanel` with three tabs over **one shared data layer**
(`ActivityStore` + `SessionStore`; stores are the single source of truth):

1. **Paths** — favorite + recent directories (OSC 7 collection + manual).
   Click opens a new terminal in that directory; context actions manage
   favorites.
2. **History** — closed-session history (and command history once shell
   integration lands): a second projection of the same store.
3. **Sessions** — the active-session tree with three classification views:
   - **Time**: today / yesterday / this week / earlier.
   - **ssh:cwd**: two-level host + cwd tree; local sessions under `local:<cwd>`.
   - **Custom**: user-defined groups with drag-and-drop and auto-group rules.
   Node actions: open (restore view), keep in background, close (→ kill),
   rename group. Every tab has a fuzzy search box (title/cwd/host/tags) over
   virtualized lists.

Classification projections are pure functions in `recoil-core`
(unit-testable); panels only subscribe to store events.

SSH profile management UI (G4): settings-window page with profile list, form
editor, group tree, and an import wizard. Settings UI (G2): separate window,
terminal / appearance / features pages, edit-and-preview with debounced
writes.

## 6. Background Residency and Tray

- woocraft `tray` feature (Linux: StatusNotifierItem via zbus).
- Window close → hide (not exit); all sessions become `Backgrounded`.
- Tray menu: show/hide window; new terminal; active-session submenu
  (per-session raise/close); quit (confirm when sessions are alive).
- Tray events (crossbeam channel) are forwarded onto the GPUI main executor.
- Single instance: DBus name claim; second launches wake the first.
- Platform risk: Wayland hide support is incomplete — an explicit capability
  probe picks the best available behavior (hide / minimize) per the G6
  matrix. X11, macOS, and Windows use native hide.

## 7. Performance and Quality Strategy

- **Hot path isolation.** Event batching is upstream (4 ms); the application
  subscribes only to low-frequency events; store events are debounced
  (500 ms) before panels update.
- **Precise notify.** Session mutations notify only entities subscribed to
  that id; panels use woocraft virtualized lists; no panel-level full
  refreshes.
- **Memory.** Scrollback clamped per config; history/path caps with LRU
  shrink; backgrounded sessions hold no view (PTY buffering only).
- **Testing.** `recoil-core` fully unit-tested (validation, projections,
  FSM); session lifecycle proven headlessly with real PTYs
  (`spawn_with_events`); UI surfaces verified through the scenario catalog.
- **i18n.** Every user-facing string is a key with complete `en-US` and
  `zh-CN` translations at every gate (ADR-0003).
- **Upstream discipline.** Shims reference upstream issues and are removed
  on landing; product semantics (close paths, grouping) are never proposed
  upstream — only generic capabilities are.
