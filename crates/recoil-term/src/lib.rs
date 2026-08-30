//! Recoil application library.
//!
//! The binary entry point is in `main.rs`; this crate exists so integration
//! tests can exercise application-facing modules through the public surface.
//! Domain logic that must run headless belongs in `recoil-core`, not here.

pub mod localization;
pub mod panels;
pub mod stores;
pub mod terminal;
#[cfg(feature = "tray")]
pub mod tray;
pub mod welcome;
pub mod workspace;

#[cfg(not(feature = "tray"))]
pub mod tray {
  /// No-op stub used when the tray feature is disabled.
  pub mod tray {
    pub fn init(_cx: &mut woocraft::gpui::App) {}
  }

  pub use tray::init;
}

use gpui::App;

// Per ADR-0003 all user-facing strings resolve through rust-i18n. The
// `i18n!` macro must be invoked at the crate root so `t!` finds the
// generated backend; locale selection is owned by woocraft.
rust_i18n::i18n!("locales", fallback = "en-us");

/// The application display name. User-facing strings are resolved through
/// rust-i18n; this identifier is locale-independent.
pub const APP_NAME: &str = "Recoil";

/// Initializes the application: stores, actions, keymap, and menus.
pub fn init(cx: &mut App) {
  localization::init(cx);
  stores::sessions::init(cx);
  workspace::bind_keys(cx);
  workspace::set_app_menu(cx);
  workspace::observe_sessions(cx);
  #[cfg(feature = "tray")]
  tray::observe_sessions(cx);
}

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
