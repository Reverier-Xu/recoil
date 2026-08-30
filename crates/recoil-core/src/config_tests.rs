use crate::{
  config::{Config, CursorShape, FeaturesConfig, TerminalConfig, ThemeMode},
  error::Error,
};

#[test]
fn defaults_are_valid() {
  let config = Config::default();
  config.validate().expect("default config must validate");
  assert_eq!(config.terminal, TerminalConfig::default());
}

#[test]
fn roundtrips_through_toml() {
  let config = Config::default();
  let text = toml::to_string(&config).expect("serialize default config");
  let parsed = Config::from_toml(&text).expect("parse serialized config");
  assert_eq!(parsed, config);
}

#[test]
fn roundtrips_a_fully_specified_document() {
  let raw = r##"
[terminal]
font-family = "JetBrains Mono"
font-fallbacks = ["Noto Sans Mono CJK SC", "Symbols Nerd Font"]
font-size = 14.0
scrolling-history = 50000
cursor-shape = "bar"
cursor-blink = false
alternate-scroll = false

[terminal.features]
hyperlink = false
osc52 = false

[theme]
mode = "dark"

[theme.terminal-palette]
black = "#000000"
bright-white = "#ffffff"
foreground = "#d0d0d0"
"##;
  let config = Config::from_toml(raw).expect("full document must parse");
  assert_eq!(config.terminal.font_family, "JetBrains Mono");
  assert_eq!(config.terminal.cursor_shape, CursorShape::Bar);
  assert!(!config.terminal.cursor_blink);
  assert!(!config.terminal.alternate_scroll);
  assert!(!config.terminal.features.hyperlink);
  assert!(!config.terminal.features.osc52);
  assert!(
    config.terminal.features.bell,
    "unset features keep defaults"
  );
  assert_eq!(config.theme.mode, ThemeMode::Dark);
  let palette = config
    .theme
    .terminal_palette
    .as_ref()
    .expect("palette parsed");
  assert_eq!(palette.black.as_deref(), Some("#000000"));
  assert_eq!(palette.bright_white.as_deref(), Some("#ffffff"));
  assert_eq!(palette.red, None, "unset palette entries stay unset");

  let serialized = toml::to_string(&config).expect("serialize full config");
  let reparsed = Config::from_toml(&serialized).expect("reparse full config");
  assert_eq!(reparsed, config);
}

#[test]
fn rejects_unknown_fields() {
  assert!(Config::from_toml("[terminal]\nnope = true\n").is_err());
  assert!(Config::from_toml("[theme]\nnope = true\n").is_err());
  assert!(Config::from_toml("[terminal.features]\nnope = true\n").is_err());
  assert!(Config::from_toml("[theme.terminal-palette]\nnope = \"#000000\"\n").is_err());
}

fn rejected(raw: &str, needle: &str) -> Error {
  match Config::from_toml(raw) {
    Err(err) => err,
    Ok(_) => panic!("configuration must be rejected: {needle}"),
  }
}

#[test]
fn rejects_out_of_range_font_size() {
  let err = rejected("[terminal]\nfont-size = 999.0\n", "font-size");
  assert!(err.to_string().contains("font-size"));
}

#[test]
fn accepts_font_size_boundaries() {
  for size in [6.0, 128.0] {
    Config::from_toml(&format!("[terminal]\nfont-size = {size}\n"))
      .unwrap_or_else(|_| panic!("boundary font-size {size} must validate"));
  }
  for size in [5.9, 128.1] {
    rejected(&format!("[terminal]\nfont-size = {size}\n"), "font-size");
  }
}

#[test]
fn rejects_blank_font_families() {
  let err = rejected("[terminal]\nfont-family = \"\"\n", "font-family");
  assert!(err.to_string().contains("font-family"));
  let err = rejected(
    "[terminal]\nfont-fallbacks = [\"Symbols Nerd Font\", \" \"]\n",
    "font-fallbacks",
  );
  assert!(err.to_string().contains("font-fallbacks"));
}

#[test]
fn accepts_unlimited_scrolling_history_sentinel() {
  let config = Config::from_toml("[terminal]\nscrolling-history = 0\n")
    .expect("scrolling-history = 0 must mean unlimited");
  assert_eq!(config.terminal.scrolling_history, 0);
}

#[test]
fn rejects_finite_scrolling_history_below_minimum() {
  for history in [1, 99, 1_000_001] {
    let err = rejected(
      &format!("[terminal]\nscrolling-history = {history}\n"),
      "scrolling-history",
    );
    assert!(err.to_string().contains("scrolling-history"));
  }
}

#[test]
fn cursor_shape_is_kebab_cased_and_checked() {
  for (raw, shape) in [
    ("block", CursorShape::Block),
    ("underline", CursorShape::Underline),
    ("bar", CursorShape::Bar),
    ("hollow", CursorShape::Hollow),
  ] {
    let config = Config::from_toml(&format!("[terminal]\ncursor-shape = \"{raw}\"\n"))
      .expect("known cursor shape must parse");
    assert_eq!(config.terminal.cursor_shape, shape);
  }
  rejected("[terminal]\ncursor-shape = \"cross\"\n", "cursor-shape");
}

#[test]
fn theme_mode_is_kebab_cased_and_checked() {
  for raw in ["light", "dark", "system"] {
    Config::from_toml(&format!("[theme]\nmode = \"{raw}\"\n"))
      .expect("known theme mode must parse");
  }
  rejected("[theme]\nmode = \"blue\"\n", "mode");
}

#[test]
fn features_default_on_and_flip_individually() {
  let config = Config::default();
  assert_eq!(
    config.terminal.features,
    FeaturesConfig {
      hyperlink: true,
      smart_select: true,
      mouse_reporting: true,
      copy_on_select: true,
      osc52: true,
      bell: true,
      bell_when_hidden_notify: true,
    }
  );
  let config = Config::from_toml("[terminal.features]\nbell = false\ncopy-on-select = false\n")
    .expect("feature overrides must parse");
  assert!(!config.terminal.features.bell);
  assert!(!config.terminal.features.copy_on_select);
  assert!(config.terminal.features.osc52, "untouched features stay on");
}

#[test]
fn palette_entries_must_be_hex_colors() {
  Config::from_toml("[theme.terminal-palette]\nred = \"#ff3b30\"\n")
    .expect("a #rrggbb color must validate");
  Config::from_toml("[theme.terminal-palette]\nred = \"ff3b30\"\n")
    .expect("a bare rrggbb color must validate");
  for bad in ["red", "#fff", "#ffff3b3000", "#zz0000"] {
    let err = rejected(
      &format!("[theme.terminal-palette]\nred = \"{bad}\"\n"),
      "terminal-palette",
    );
    assert!(
      err.to_string().contains("theme.terminal-palette.red"),
      "error names the offending entry: {err}"
    );
  }
}

#[test]
fn exports_a_json_schema_describing_the_surface() {
  let schema = Config::json_schema().expect("schema export must succeed");
  for key in [
    "font-family",
    "font-fallbacks",
    "font-size",
    "scrolling-history",
    "cursor-shape",
    "cursor-blink",
    "alternate-scroll",
    "features",
    "hyperlink",
    "osc52",
    "theme",
    "mode",
    "terminal-palette",
    "bright-white",
  ] {
    assert!(
      schema.contains(&format!("\"{key}\"")),
      "schema must mention {key}"
    );
  }
}

/// The checked-in schema artifact is generated from `Config::json_schema()`
/// (`cargo run -p recoil-core --example export-schema`); a drift between
/// the two fails here instead of surprising downstream consumers. The
/// comparison parses both sides because schemars' property ordering depends
/// on feature unification; the schema's semantics do not.
#[test]
fn checked_in_schema_matches_the_export() {
  let exported = Config::json_schema().expect("schema export must succeed");
  let artifact = include_str!("../schema/config.schema.json");
  let exported: serde_json::Value =
    serde_json::from_str(&exported).expect("the export is valid json");
  let artifact: serde_json::Value =
    serde_json::from_str(artifact).expect("the artifact is valid json");
  assert_eq!(
    exported, artifact,
    "schema/config.schema.json is stale; regenerate it with \
     `cargo run -p recoil-core --example export-schema > crates/recoil-core/schema/config.schema.json`"
  );
}
