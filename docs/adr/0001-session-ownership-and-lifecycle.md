---
id: ADR-0001
title: Session ownership independent of views
status: accepted
date: 2026-08-30
deciders: recoil maintainers
---

# Session Ownership Independent of Views

## Context

A terminal session (PTY plus emulator state) outlives any particular view of
it. Tabs close, windows hide, dock trees remove panels, and yet the shell
inside the session keeps running. `woocraft-terminal` already models this:
`TerminalSession` is a cheap, cloneable handle whose lifecycle is independent
of the GPUI view, and `TerminalView` re-snapshots content from the session
during prepaint. The application must decide who owns the handle and what the
close affordances mean; ad-hoc ownership in views would make background
sessions, restoration, and tray residency impossible to specify.

## Decision Drivers

- Users expect IDE semantics: closing a tab is not closing a process.
- Session metadata (title, cwd, ssh host, creation time) must survive view
  churn and power the left-dock panels and tray menu.
- Exactly one owner makes lifecycle reasoning, testing, and persistence
  tractable.
- The three close affordances (tab, dock tree, root process exit) must have
  unambiguous, testable semantics.

## Decision

### Ownership

The `SessionStore` — a GPUI global — is the only owner of
`TerminalSession` handles. Views, tabs, dock panels, and windows are
observers created on demand from a `SessionId`. No other component may hold a
long-lived session handle. Dropping a view never affects the session.

### Lifecycle State Machine

```text
Spawning ──▶ Active ──▶ Backgrounded ──▶ Active (restore)
              │  ▲            │
              ▼  │            ▼
           Exited ◀──────────┘ (kill or root exit, from any state)
              │
              ▼
            Reaped (entry removed; tabs/tree/tray notified)
```

- `Spawning`: the PTY is being created.
- `Active`: at least one view shows the session.
- `Backgrounded`: no view exists; the PTY is alive and controllable.
- `Exited`: the root process ended (or `kill()` was requested); the entry is
  retained until subscribers are notified, then reaped.

### Close-Path Semantics

1. **Root process exit** (`ChildExit`/`Exit` events): the entry transitions
   to `Exited`, subscribers (tab bar, sessions panel, tray) are notified, the
   entry is reaped, and any attached tab closes. Idempotent.
2. **Dock tree close** (sessions panel action): `kill()` on the session —
   including backgrounded ones — followed by path 1. This is the only
   user affordance that terminates a session without a view.
3. **Tab close**: drop the view; the session moves `Active → Backgrounded`.
   The PTY is untouched.
4. **Window close / UI exit**: every view is dropped and every session moves
   to `Backgrounded`; the process and tray stay resident.

### View Restoration

Opening a backgrounded session from the dock tree or tray creates a new
`TerminalView` from the stored handle. Scrollback, titles, and emulator state
come from the session; presentation metadata (ssh kind, tags, group) comes
from the store. If upstream `TerminalView` needs an explicit restore
initializer, that is a registered upstream item, not a local workaround.

### Event Hygiene

High-frequency terminal content events never leave the terminal view's event
pump. Only low-frequency metadata events (title, cwd, exit) cross into the
store, debounced per the decision register.

## Consequences

- The sessions panel, tray menu, and tab bar are projections of one store and
  cannot disagree.
- Testing close semantics is possible headlessly with real PTYs and no
  window.
- Killing the application process still kills all sessions (the OS owns the
  PTY); background persistence is within-process only, and G5 documents the
  restart story honestly (reopen suggestions, not resurrection).
- View restoration depends on upstream snapshot behavior being verified at
  G1; any gap is a registered upstream task.
