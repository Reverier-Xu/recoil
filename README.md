# Recoil

An IDE-style terminal emulator built on [woocraft](https://github.com/Reverier-Xu/woocraft)
and GPUI. Terminal-first and performance-first: sessions outlive tabs and
windows, SSH connections are managed as profiles, and active sessions are
organized by time, `ssh:cwd`, or custom trees.

Status: early development. See [docs/DESIGN.md](docs/DESIGN.md) for the
architecture, [docs/roadmap.md](docs/roadmap.md) for the program, and
[AGENTS.md](AGENTS.md) for the development conventions.

## Development

```bash
cargo +nightly fmt --all   # formatting (nightly rustfmt rules)
taplo fmt                  # TOML formatting
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
scripts/validate-planning-docs.sh
```

Every change must pass the full quality suite in [AGENTS.md](AGENTS.md); CI
enforces the same gates plus the MSRV (1.96), stable, macOS, Windows, and
feature-powerset builds.
