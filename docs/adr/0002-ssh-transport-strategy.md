---
id: ADR-0002
title: SSH transport via the ssh binary
status: accepted
date: 2026-08-30
deciders: recoil maintainers
---

# SSH Transport via the ssh Binary

## Context

Recoil must manage SSH connections as first-class sessions: profiles with
hosts, ports, users, authentication, and jump chains; grouping; and status
metadata. A terminal has two realistic transport strategies:

1. Wrap the user's `ssh` binary: the profile compiles to a command line and
   the PTY runs `ssh`.
2. Embed an SSH library (russh, libssh2) and implement the protocol in
   process.

Zed's remote story and mainstream terminals (iTerm2, Windows Terminal,
Konsole) treat strategy 1 as the baseline; it inherits the user's agent,
known_hosts, config, ProxyCommand, and GSSAPI behavior for free, and the
security boundary stays where the user already put it.

## Decision Drivers

- Correctness of auth (agent, keys, certificates) is hard to reimplement.
- Users already curate `~/.ssh/config`; import must interoperate with it.
- A terminal's value is not in reimplementing SSH cryptography.
- Future needs (connection pooling, SFTP browsing, port-forward UI) are real
  but speculative; the design must not pay for them now.

## Decision

Recoil uses the ssh binary as its SSH transport. `SshProfile` data compiles
to `ssh` arguments; the PTY child is the ssh process, so root-process-exit
semantics, kill semantics, and the session state machine apply unchanged.

- A `SshTransport` trait owns the compilation from profile to spawn options.
  The ssh binary implementation ships in G4; embedded implementations are a
  future ADR, not a present need.
- Agent auth is preferred. Passphrases and passwords are never stored;
  prompts flow through the PTY like any interactive program.
- Jump chains compile to `-J` (or explicit `ProxyJump` semantics); per-hop
  key paths, ports, and users are supported.
- `~/.ssh/config` import is a strict parse of host blocks into profile data;
  unknown directives are preserved as comments. Imported values are data,
  never shell text. This import must not become a general ssh config editor
  that round-trips the user's file wholesale.
- Session metadata records how a session was born (`SessionOrigin`) and,
  separately, what is currently observed inside it (`SessionObservation`).
  Users ssh into hosts and exit back to local shells at will, so the ssh
  state and the working directory are dynamic observations that follow
  terminal behavior — never fixed attributes. Classification views group by
  the current observation.

## Consequences

- Recoil requires the `ssh` binary at runtime for SSH features; absence is a
  surfaced, actionable error, not a silent failure.
- Connection-level errors (timeouts, auth failures) appear inside the
  terminal, as with any ssh client; the UI surfaces connection state through
  the session metadata it can observe (spawn success, root exit).
- Mosh, telnet, serial, and WSL transports fit the same trait but are
  non-goals until accepted by ADR.
