//! Headless domain model for Recoil.
//!
//! This crate holds everything that must be testable without a GPU, a window,
//! or a running GPUI application:
//!
//! - [`config`]: user configuration (terminal, theme, features) as serde data
//!   with validation and TOML persistence.
//! - SSH profiles and session metadata (added by G4/G5 tasks).
//! - Classification projections over sessions and activity records (added by G5
//!   tasks).
//!
//! GUI code lives in the `recoil-term` bin crate and must never be imported
//! here.

// Production code must not panic on results; unit tests are exempt because
// panicking is the assertion mechanism there. See AGENTS.md.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod config;
pub mod error;
