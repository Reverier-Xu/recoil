# AGENTS.md

## Project Scope

`recoil` is a Rust 2024 workspace shipping an IDE-style terminal emulator built
on GPUI. All GUI types enter the workspace exclusively through the `woocraft`
git dependency and its re-exports (`woocraft::gpui`,
`woocraft_terminal::alacritty_terminal`); never add direct git dependencies on
`gpui`, `zed`, or `alacritty`. Keep changes minimal, atomic, and aligned with
the planning system under `docs/`.

## Language Policy

- Development artifacts — code, comments, docs, commit messages, issue text —
  are written in English.
- User-facing application strings are never hardcoded. Every string shown in
  the UI is a rust-i18n key resolved through `woocraft::t` / the workspace
  `rust-i18n` setup. Tier-0 locales are `en-US` and `zh-CN`; both must ship
  complete translations for every key at every gate. Adding a key without
  both translations is a release blocker.
- Locale files live in `crates/recoil-term/locales/` (`en-US.yml`,
  `zh-CN.yml`).

## Repository Structure

- `crates/recoil-core/`: headless domain model (configuration, SSH profiles,
  session metadata, classification projections, persistence). No GPUI
  dependency; everything here must be testable without a display.
- `crates/recoil-term/`: the GPUI application binary (stores, panels, views,
  actions, tray integration) and the locale files.
- `docs/`: architecture design, roadmap, development gates, implementation
  plan, planning manifests (TOML), and ADRs.
- `scripts/`: planning validation and per-task verification entry points.
- `.github/workflows/quality_check.yml`: required CI quality gates.
- `rustfmt.toml` / `taplo.toml`: formatting configuration.

Keep unit tests next to the code they validate in `#[cfg(test)]` modules.
Add integration tests under `crates/*/tests/` only when behavior must be
exercised through the public API of a crate. Headless terminal lifecycle
tests use `woocraft_terminal::TerminalSession` directly, never a window.

## Implementation Principles

- Prefer long-term correctness over short-lived workarounds.
- Choose the simplest design that preserves clear ownership and future
  changeability.
- Reuse existing modules, `woocraft` widgets, and helpers before introducing
  abstractions.
- Keep each change atomic and avoid unrelated refactors.
- Do not add dependencies unless they remove meaningful complexity or provide
  required domain behavior. New dependencies require a plan amendment or ADR.
- Forbid `unsafe` code unless the user explicitly approves it and all safety
  invariants are documented.
- Do not use `unwrap()` or `expect()` in production code (enforced by
  `cfg_attr(not(test), deny(...))` per crate). Unit tests may use `expect()`.
- All logging and diagnostics must go through the `tracing` crate. Never add
  `println!`, `eprintln!`, `print!`, or `dbg!` — including during debugging.
  Temporary debug prints left in code are release blockers.
- GPUI discipline: never block the main thread; keep terminal I/O on the
  session event pumps; `cx.notify()` only the entities that actually changed;
  terminal `Wakeup` events must never invalidate stores or dock panels.
- Secrets discipline: passwords, passphrases, and key material never enter
  config files, state files, logs, or tracing output.
- UI typography is uniform: one font family and one font size (16 px by
  default, configured centrally) shared by the UI and the terminal. Never
  use `text_xs`/`text_sm`/`text_lg`/`text_xl` or any other per-element size
  override. Express emphasis with bold (`font_semibold`) and de-emphasis
  with theme colors/opacity (`muted_foreground`), never with size.
- Use the standard tools for standard code operations: read files with the
  read tool, locate with grep/rg, and edit with the edit tool. Never edit,
  generate, or patch project files through python/shell one-off scripts
  (`python -c`, `sed -i`, `awk` in-place, heredoc rewrites, …). Script-driven
  edits hide what changed, cannot prove how many places a pattern matched,
  and defeat review — treat a script-based edit like an unreviewable commit.
  Reserve python (or ad-hoc scripts) for complex behavior testing, numerical
  computation, and data analysis where they are genuinely the right tool,
  and never let them write back into the repository.

## Planning System

Long-running work is governed by the documents under `docs/`:

- `docs/DESIGN.md` — product and architecture design (the terminal-state
  reference).
- `docs/roadmap.md` — product contract, boundaries, milestones, traceability.
- `docs/development-gates.md` — gate protocol and acceptance per gate.
- `docs/implementation-plan.md` — the task table (`T-Gxx-yy` IDs).
- `docs/task-verification.toml` — per-task verification argv registry.
- `docs/scenario-catalog.toml`, `docs/evidence-impact.toml`,
  `docs/threat-model.toml`, `docs/decision-register.toml` — supporting
  manifests.
- `docs/adr/` — accepted architecture decisions.

A task starts only after its verification argv is registered in
`docs/task-verification.toml`. Run it with:

```bash
scripts/verify-task.sh T-Gxx-yy
```

Structural consistency of the planning documents is enforced by:

```bash
scripts/validate-planning-docs.sh
```

## Formatting

Rust formatting uses the nightly toolchain because `rustfmt.toml` enables
unstable options:

```bash
cargo +nightly fmt --all
cargo +nightly fmt --all -- --check
```

TOML formatting uses Taplo:

```bash
taplo fmt
taplo fmt --check
```

Do not hand-format around these tools. Run both checks after changing Rust or
TOML files.

## Quality Gates

Every change must pass with zero warnings:

```bash
taplo fmt --check
cargo +nightly fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
scripts/validate-planning-docs.sh
```

When `.github/workflows/` changes, validate the workflow with `act` when
Docker is available. If local workflow execution is unavailable, run every CI
command locally and verify the pushed GitHub Actions run before declaring
completion.

## Git Conventions

- Use one logical change per commit; commit early and push in small steps.
- Use a short, lowercase, imperative summary prefixed by a valid gitmoji
  shortcode.
- Use a bulleted commit body when details are needed.
- Use short kebab-case branch names; the default branch is `main`.
- Never push directly to `main` without a green local quality suite; CI must
  pass on every pushed commit.

Example:

```text
:wrench: initialize project quality gates

- add taplo and nightly rustfmt configuration
- enforce formatting, linting, and tests in ci
```

## Completion Checklist

Before committing or handing work back:

1. Run all formatting and quality gates listed above.
2. Fix every warning, error, and formatting diff.
3. Confirm workflow changes are syntactically valid and locally exercised
   where possible.
4. Confirm every new user-facing string exists in both tier-0 locales.
5. Remove temporary artifacts and leave only intentional changes.
6. Review the final diff for accidental API, dependency, or metadata changes.
