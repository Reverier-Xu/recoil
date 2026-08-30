//! User configuration model.
//!
//! The full G2 surface lives here: `[terminal]` appearance and behavior,
//! `[terminal.features]` switches, and `[theme]` mode with an optional
//! terminal palette override. Everything is serializable pure data with
//! validation and a JSON Schema export; hot reload and the settings UI
//! (G2 follow-up tasks) build on these types without changing them.

use std::path::PathBuf;

use schemars::JsonSchema;
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

/// Cursor shapes for the terminal caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CursorShape {
  /// Filled block.
  Block,
  /// Underline below the cell.
  Underline,
  /// Vertical bar at the cell edge.
  Bar,
  /// Hollow (outlined) block.
  Hollow,
}

/// Feature switches under `[terminal.features]`. Every extension that
/// touches terminal behavior is gated here (ADR: configuration is data);
/// a disabled switch must remove all of the feature's hot-path work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct FeaturesConfig {
  /// OSC 8 hyperlinks: hover underline, modifier-click open, copy-link.
  pub hyperlink: bool,
  /// Smart selection: word characters, multi-click granularity, URL
  /// fallback over visible rows.
  pub smart_select: bool,
  /// Application mouse-mode passthrough to terminal programs.
  pub mouse_reporting: bool,
  /// Copy to the clipboard on selection.
  pub copy_on_select: bool,
  /// Allow programs to read and write the clipboard via OSC 52 (THR-001).
  pub osc52: bool,
  /// React to the terminal bell (tab highlight).
  pub bell: bool,
  /// Escalate the bell to a notification while the window is hidden.
  pub bell_when_hidden_notify: bool,
}

impl Default for FeaturesConfig {
  fn default() -> Self {
    Self {
      hyperlink: true,
      smart_select: true,
      mouse_reporting: true,
      copy_on_select: true,
      osc52: true,
      bell: true,
      bell_when_hidden_notify: true,
    }
  }
}

/// Terminal appearance and behavior settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
  /// The caret shape.
  pub cursor_shape: CursorShape,
  /// Whether the caret blinks.
  pub cursor_blink: bool,
  /// Send arrow keys instead of scrolling when the alternate screen is
  /// active.
  pub alternate_scroll: bool,
  /// Switchable terminal extensions.
  pub features: FeaturesConfig,
}

impl Default for TerminalConfig {
  fn default() -> Self {
    Self {
      font_family: "Maple Mono".to_owned(),
      font_fallbacks: Vec::new(),
      font_size: 16.0,
      scrolling_history: 10_000,
      cursor_shape: CursorShape::Block,
      cursor_blink: true,
      alternate_scroll: true,
      features: FeaturesConfig::default(),
    }
  }
}

/// How the application picks light or dark colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
  /// Always light.
  Light,
  /// Always dark.
  Dark,
  /// Follow the operating system.
  System,
}

/// A terminal palette override: the 16 ANSI colors plus foreground,
/// background, cursor, and selection. Colors are `#rrggbb` strings; unset
/// entries derive from the active woocraft theme.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct TerminalPalette {
  pub foreground: Option<String>,
  pub background: Option<String>,
  pub cursor: Option<String>,
  pub selection: Option<String>,
  pub black: Option<String>,
  pub red: Option<String>,
  pub green: Option<String>,
  pub yellow: Option<String>,
  pub blue: Option<String>,
  pub magenta: Option<String>,
  pub cyan: Option<String>,
  pub white: Option<String>,
  pub bright_black: Option<String>,
  pub bright_red: Option<String>,
  pub bright_green: Option<String>,
  pub bright_yellow: Option<String>,
  pub bright_blue: Option<String>,
  pub bright_magenta: Option<String>,
  pub bright_cyan: Option<String>,
  pub bright_white: Option<String>,
}

/// Theme settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct ThemeConfig {
  /// Light/dark selection.
  pub mode: ThemeMode,
  /// Optional terminal palette override; defaults derive from the active
  /// woocraft theme.
  pub terminal_palette: Option<TerminalPalette>,
}

impl Default for ThemeConfig {
  fn default() -> Self {
    Self {
      mode: ThemeMode::System,
      terminal_palette: None,
    }
  }
}

/// Top-level user configuration document (`config.toml`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
  pub terminal: TerminalConfig,
  pub theme: ThemeConfig,
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
    if self.terminal.font_family.trim().is_empty() {
      return Err(Error::Validation(
        "terminal.font-family must not be empty".to_owned(),
      ));
    }
    if self
      .terminal
      .font_fallbacks
      .iter()
      .any(|family| family.trim().is_empty())
    {
      return Err(Error::Validation(
        "terminal.font-fallbacks must not contain empty entries".to_owned(),
      ));
    }
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
    if let Some(palette) = &self.theme.terminal_palette {
      validate_palette(palette)?;
    }
    Ok(())
  }

  /// The JSON Schema (draft 2020-12) for `config.toml`. The checked-in
  /// artifact at `crates/recoil-core/schema/config.schema.json` is
  /// regenerated from this function and guarded against drift by a test.
  pub fn json_schema() -> Result<String, Error> {
    let schema = schemars::schema_for!(Config);
    serde_json::to_string_pretty(&schema).map_err(Error::Serialize)
  }
}

/// Validates every set palette entry as a `#rrggbb` color.
fn validate_palette(palette: &TerminalPalette) -> Result<(), Error> {
  let value = serde_json::to_value(palette).map_err(Error::Serialize)?;
  let Some(entries) = value.as_object() else {
    return Ok(());
  };
  for (key, value) in entries {
    let Some(color) = value.as_str() else {
      continue;
    };
    if !is_hex_color(color) {
      return Err(Error::Validation(format!(
        "theme.terminal-palette.{key} must be a #rrggbb color"
      )));
    }
  }
  Ok(())
}

/// A `#rrggbb` (or bare `rrggbb`) hex color.
fn is_hex_color(color: &str) -> bool {
  let hex = color.strip_prefix('#').unwrap_or(color);
  hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
