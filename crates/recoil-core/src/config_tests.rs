use crate::{
  config::{Config, TerminalConfig},
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
fn rejects_unknown_fields() {
  assert!(Config::from_toml("[terminal]\nnope = true\n").is_err());
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
