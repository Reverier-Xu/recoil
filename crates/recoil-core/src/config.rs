//! User configuration model.
//!
//! G2 owns the full surface (hot reload, schema export, settings UI wiring).
//! This module establishes the serialization shape and validation rules that
//! later tasks extend without breaking.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Mirror of `woocraft_terminal::MAX_SCROLLING_HISTORY`.
///
/// `recoil-core` must stay GUI-free, so the cap is duplicated here with a
/// cross-check test in the `recoil-term` crate that imports the real
/// `woocraft-terminal` constant.
pub const MAX_SCROLLING_HISTORY: usize = 1_000_000;

/// Minimum in-memory scrollback length when a finite history is configured.
pub const MIN_SCROLLING_HISTORY: usize = 100;

/// Sentinel stored in [`TerminalConfig::scrolling_history`] meaning
/// "unlimited": scrollback is paged to a disk-backed cache (konsole/kitty
/// style) instead of being bounded in memory.
pub const UNLIMITED_SCROLLING_HISTORY: usize = 0;

/// Terminal appearance and behavior settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct TerminalConfig {
  /// Primary monospace font family.
  pub font_family: String,
  /// Ordered fallback families applied after the primary family.
  pub font_fallbacks: Vec<String>,
  /// Base font size in CSS pixels (px), matching the woocraft theme default
  /// of 16 px. Never a point value.
  pub font_size: f64,
  /// Scrollback history length in lines.
  ///
  /// - `0` ([`UNLIMITED_SCROLLING_HISTORY`]) means unlimited: scrollback is
  ///   kept in a disk-backed cache instead of memory.
  /// - Any other value is a finite in-memory history in lines, clamped by the
  ///   terminal backend to [`MAX_SCROLLING_HISTORY`].
  pub scrolling_history: usize,
}

impl Default for TerminalConfig {
  fn default() -> Self {
    Self {
      font_family: "Maple Mono".to_owned(),
      font_fallbacks: Vec::new(),
      font_size: 16.0,
      scrolling_history: 10_000,
    }
  }
}

/// Top-level user configuration document (`config.toml`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
  pub terminal: TerminalConfig,
}

impl Config {
  /// Loads and validates a configuration document from `path`.
  pub fn load(path: &PathBuf) -> Result<Self, Error> {
    let raw = std::fs::read_to_string(path).map_err(Error::Io)?;
    Self::from_toml(&raw)
  }

  /// Parses and validates a configuration document from TOML text.
  pub fn from_toml(raw: &str) -> Result<Self, Error> {
    let config: Self = toml::from_str(raw).map_err(Error::Parse)?;
    config.validate()?;
    Ok(config)
  }

  /// Validates semantic constraints that the deserializer cannot express.
  pub fn validate(&self) -> Result<(), Error> {
    if !(6.0..=128.0).contains(&self.terminal.font_size) {
      return Err(Error::Validation(
        "terminal.font-size out of range".to_owned(),
      ));
    }
    let history = self.terminal.scrolling_history;
    if history != UNLIMITED_SCROLLING_HISTORY
      && !(MIN_SCROLLING_HISTORY..=MAX_SCROLLING_HISTORY).contains(&history)
    {
      return Err(Error::Validation(
        "terminal.scrolling-history must be 0 (unlimited) or between 100 and 1000000".to_owned(),
      ));
    }
    Ok(())
  }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
