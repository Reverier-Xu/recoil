---
id: ADR-0003
title: English development and tier-0 i18n
status: accepted
date: 2026-08-30
deciders: recoil maintainers
---

# English Development and Tier-0 i18n

## Context

Recoil is developed in English-speaking tooling (compiler diagnostics, Rust
ecosystem, git history) while its users include a large Simplified-Chinese
audience. Mixed-language artifacts rot: half-translated docs diverge from
code, and hardcoded UI strings make localization an afterthought that never
happens.

## Decision

### Development Language

All development artifacts are written in English: code, identifiers,
comments, documentation under `docs/`, commit messages, and issue text. This
includes the planning system. Chinese may appear in conversations, reviews,
and user-facing prose, but not in committed development artifacts.

### Application i18n

Every user-facing string is a rust-i18n key. No user-visible literal may be
hardcoded in Rust source; a string that appears in the UI exists in both
tier-0 locale files in the same change.

- Tier-0 locales: `en-US` and `zh-CN`. Both are complete at every gate; a key
  missing from either is a release blocker.
- Locale files live in `crates/recoil-term/locales/` as `en-US.yml` and
  `zh-CN.yml`, resolved through the `rust-i18n` integration that woocraft
  also uses (`woocraft::t` for woocraft-owned strings, the application
  binding for recoil-owned strings).
- Translation keys are namespaced by surface (`terminal.*`, `panels.*`,
  `settings.*`, `tray.*`, `ssh.*`).
- Additional locales are tier-1: community-translated, reviewed but not
  release-blocking; missing tier-1 keys fall back to `en-US`.

### Enforcement

- G7 runs a completeness audit (every key in both files) and a grep gate for
  hardcoded user-facing strings.
- Every UI task's scenario catalog entry carries a tier-0 completeness
  acceptance where it adds strings.

## Consequences

- Slightly more ceremony per string change; in exchange, the application is
  shippable to both audiences at every gate instead of localizing under
  deadline pressure.
- The woocraft locale tooling is shared, so framework strings follow the
  same tier-0 commitment.
