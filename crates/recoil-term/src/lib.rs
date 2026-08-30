//! Recoil application library.
//!
//! The binary entry point is in `main.rs`; this crate exists so integration
//! tests can exercise application-facing modules through the public surface.
//! Domain logic that must run headless belongs in `recoil-core`, not here.

/// The application display name. User-facing strings are resolved through
/// rust-i18n; this identifier is locale-independent.
pub const APP_NAME: &str = "Recoil";

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn app_name_is_stable() {
    assert_eq!(APP_NAME, "Recoil");
  }

  /// Guards the duplicated constant in `recoil-core` against upstream drift.
  #[test]
  fn scrolling_history_cap_matches_woocraft_terminal() {
    assert_eq!(
      recoil_core::config::MAX_SCROLLING_HISTORY,
      woocraft_terminal::MAX_SCROLLING_HISTORY
    );
  }
}
